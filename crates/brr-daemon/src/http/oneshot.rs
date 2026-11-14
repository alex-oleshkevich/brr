use std::{collections::HashMap, sync::Arc};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Query, State, rejection::BytesRejection},
    http::{HeaderMap, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
    routing::{MethodFilter, MethodRouter, on},
};
use brr_core::{
    Error,
    capture::{CaptureParams, OneshotKind},
};
use serde_json::Value;

use super::ops::{ApiError, assemble, body_bytes, with_logged_client};
use crate::{
    app::App,
    internal,
    ops::{self, OpRegistration},
    services::{artifacts::Delivery, oneshot},
};

const CHARSET: &str = "; charset=utf-8";

const KINDS: [(OneshotKind, &str); 6] = [
    (OneshotKind::Screenshot, "screenshot"),
    (OneshotKind::Pdf, "pdf"),
    (OneshotKind::Mhtml, "mhtml"),
    (OneshotKind::Markdown, "markdown"),
    (OneshotKind::Content, "content"),
    (OneshotKind::Links, "links"),
];

// The registry op of the same kind owns the input schema, so a GET query is coerced the way
// every other endpoint's is.
pub fn routes() -> Router<Arc<App>> {
    let mut router = Router::new();

    for (kind, name) in KINDS {
        if let Some(op) = ops::find(&format!("oneshot.{name}")) {
            router = router.route(&format!("/v1/{name}"), route(kind, op));
        }
    }

    router
}

fn route(kind: OneshotKind, op: &'static OpRegistration) -> MethodRouter<Arc<App>> {
    on(
        MethodFilter::GET.or(MethodFilter::POST),
        move |State(app): State<Arc<App>>,
              Query(query): Query<Vec<(String, String)>>,
              headers: HeaderMap,
              body: Result<Bytes, BytesRejection>| async move {
            serve(app, kind, op, query, headers, body).await
        },
    )
}

async fn serve(
    app: Arc<App>,
    kind: OneshotKind,
    op: &'static OpRegistration,
    query: Vec<(String, String)>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, ApiError> {
    let input = assemble(op, HashMap::new(), query, &body_bytes(body)?)?;
    let params: CaptureParams = ops::parse(input)?;

    let delivery = with_logged_client(app, &headers, op.spec.name, |ctx| async move {
        oneshot::run(&ctx.app, &ctx.client, kind, params).await
    })
    .await?;

    Ok(respond(delivery)?)
}

// An inline result is the bytes themselves; a stored one is described as JSON without its empty
// fields.
fn respond(delivery: Delivery) -> Result<Response, Error> {
    if delivery.url.is_some() {
        let Value::Object(mut fields) = serde_json::to_value(&delivery).map_err(internal)? else {
            return Err(internal("a delivery is a JSON object"));
        };
        fields.retain(|_, value| !value.is_null());

        return Ok(Json(fields).into_response());
    }

    let content_type = if delivery.mime.starts_with("text/") {
        format!("{}{CHARSET}", delivery.mime)
    } else {
        delivery.mime
    };
    let bytes = delivery
        .inline_base64
        .map(|file| file.0)
        .unwrap_or_default();

    Ok(([(CONTENT_TYPE, content_type)], bytes).into_response())
}
