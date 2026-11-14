use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, OnceLock},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State, rejection::BytesRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
    routing::{MethodFilter, on, post},
};
use brr_core::{ClientKind, Error, Method};
use serde_json::{Map, Value};

use super::MAX_REQUEST_BYTES;
use crate::{
    app::{App, Ctx},
    mcp::inline_schema,
    ops::{self, OpRegistration},
    wire::{CLI_KIND, CLIENT_HEADER, CLIENT_KIND_HEADER},
};

pub struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(error: Error) -> ApiError {
        ApiError(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let error = self.0;
        let status =
            StatusCode::from_u16(error.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Json(error.body())).into_response();

        if let Error::CapExceeded {
            retry_after: Some(seconds),
            ..
        } = error
        {
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from(seconds));
        }

        response
    }
}

pub fn routes() -> Router<Arc<App>> {
    let mut router = Router::new().route("/v1/ops/{name}", post(by_name));

    for op in ops::all() {
        let method = op.spec.http.method;
        let filter = match method {
            Method::Get => MethodFilter::GET,
            Method::Post => MethodFilter::POST,
            Method::Put => MethodFilter::PUT,
            Method::Patch => MethodFilter::PATCH,
            Method::Delete => MethodFilter::DELETE,
        };

        router = router.route(
            op.spec.http.path,
            on(
                filter,
                move |State(app): State<Arc<App>>,
                      Path(params): Path<HashMap<String, String>>,
                      Query(query): Query<Vec<(String, String)>>,
                      headers: HeaderMap,
                      body: Result<Bytes, BytesRejection>| async move {
                    let input = assemble(op, params, query, &body_bytes(body)?)?;

                    invoke(app, op, &headers, input).await
                },
            ),
        );
    }

    router
}

// An oversized body is answered with the JSON error body, like every other failure.
pub(super) fn body_bytes(body: Result<Bytes, BytesRejection>) -> Result<Bytes, ApiError> {
    body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            return ApiError(Error::RequestTooLarge {
                limit_bytes: MAX_REQUEST_BYTES as u64,
            });
        }

        ApiError(Error::InvalidArgument {
            field: "body".to_string(),
            message: rejection.body_text(),
        })
    })
}

pub fn client_header(headers: &HeaderMap) -> Result<Option<&str>, Error> {
    headers
        .get(CLIENT_HEADER)
        .map(|value| {
            value.to_str().map_err(|_| Error::InvalidArgument {
                field: CLIENT_HEADER.to_string(),
                message: "must be plain text".to_string(),
            })
        })
        .transpose()
}

async fn by_name(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<Value>, ApiError> {
    let op = ops::find(&name).ok_or_else(|| Error::NotFound {
        what: format!("operation {name}"),
    })?;
    let input = Value::Object(body_object(&body_bytes(body)?)?);

    invoke(app, op, &headers, input).await
}

async fn invoke(
    app: Arc<App>,
    op: &'static OpRegistration,
    headers: &HeaderMap,
    input: Value,
) -> Result<Json<Value>, ApiError> {
    with_client(app, headers, |ctx| ops::dispatch(ctx, op, input))
        .await
        .map(Json)
}

// Runs a call as the Client the request names, or as an implicit one that lasts for the call.
pub(super) async fn with_client<T, Fut>(
    app: Arc<App>,
    headers: &HeaderMap,
    call: impl FnOnce(Ctx) -> Fut,
) -> Result<T, ApiError>
where
    Fut: Future<Output = Result<T, Error>>,
{
    if let Some(header) = client_header(headers)? {
        let client = app.clients.resolve_rest(&app, header).await?;

        return Ok(call(Ctx { app, client }).await?);
    }

    let kind = match headers.get(CLIENT_KIND_HEADER).map(HeaderValue::as_bytes) {
        Some(value) if value == CLI_KIND.as_bytes() => ClientKind::Cli,
        _ => ClientKind::Rest,
    };
    // The guard ends the Client on drop too, so a request dropped mid-call still releases its Leases.
    let guard = app.clients.implicit(&app, kind);
    let ctx = Ctx {
        app: Arc::clone(&app),
        client: guard.client.clone(),
    };

    let result = call(ctx).await;
    guard.finish().await;

    Ok(result?)
}

// For routes that bypass `ops::dispatch`: the same panic catch and `op` log line.
pub(super) async fn with_logged_client<T, Fut>(
    app: Arc<App>,
    headers: &HeaderMap,
    name: &str,
    call: impl FnOnce(Ctx) -> Fut,
) -> Result<T, ApiError>
where
    Fut: Future<Output = Result<T, Error>>,
{
    with_client(app, headers, |ctx| {
        let client_id = ctx.client.client_id.clone();

        async move { ops::logged(name, &client_id, call(ctx)).await }
    })
    .await
}

pub(super) fn assemble(
    op: &OpRegistration,
    params: HashMap<String, String>,
    query: Vec<(String, String)>,
    body: &[u8],
) -> Result<Value, Error> {
    let mut input = match op.spec.http.method {
        Method::Get | Method::Delete => Map::new(),
        Method::Post | Method::Put | Method::Patch => body_object(body)?,
    };
    // On a body method the query fills in what the body leaves out, e.g. PUT ...?format=netscape.
    for (name, value) in query {
        let value = scalar(value, property_schema(op, &name));
        input.entry(name).or_insert(value);
    }

    input.extend(
        params
            .into_iter()
            .map(|(name, value)| (name, Value::String(value))),
    );

    Ok(Value::Object(input))
}

fn body_object(body: &[u8]) -> Result<Map<String, Value>, Error> {
    if body.is_empty() {
        return Ok(Map::new());
    }

    let invalid = |message: String| Error::InvalidArgument {
        field: "body".to_string(),
        message,
    };

    match serde_json::from_slice(body) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(_) => Err(invalid("expected a JSON object".to_string())),
        Err(e) => Err(invalid(format!("not valid JSON: {e}"))),
    }
}

// Query values are text; the op's input schema says which of them are numbers or booleans.
fn property_schema(op: &OpRegistration, name: &str) -> Option<&'static Value> {
    static PROPERTIES: OnceLock<HashMap<&'static str, Value>> = OnceLock::new();

    PROPERTIES
        .get_or_init(|| {
            ops::all()
                .iter()
                .map(|op| {
                    let schema = inline_schema(op.spec.input_schema);
                    (
                        op.spec.name,
                        schema.get("properties").cloned().unwrap_or(Value::Null),
                    )
                })
                .collect()
        })
        .get(op.spec.name)?
        .get(name)
}

fn declares_type(schema: &Value, wanted: &[&str]) -> bool {
    match schema.get("type") {
        Some(Value::String(kind)) if wanted.contains(&kind.as_str()) => return true,
        Some(Value::Array(kinds))
            if kinds
                .iter()
                .any(|kind| kind.as_str().is_some_and(|kind| wanted.contains(&kind))) =>
        {
            return true;
        }
        _ => {}
    }

    ["anyOf", "oneOf"].iter().any(|key| {
        schema
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|branches| branches.iter().any(|branch| declares_type(branch, wanted)))
    })
}

fn scalar(text: String, schema: Option<&Value>) -> Value {
    let Some(schema) = schema else {
        return Value::String(text);
    };

    match serde_json::from_str(&text) {
        Ok(number @ Value::Number(_)) if declares_type(schema, &["integer", "number"]) => number,
        Ok(flag @ Value::Bool(_)) if declares_type(schema, &["boolean"]) => flag,
        _ => Value::String(text),
    }
}

#[cfg(test)]
mod tests;
