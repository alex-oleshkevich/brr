use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use axum::{
    Json, Router,
    body::{self, Body},
    extract::{Query, Request},
    http::{StatusCode, request::Parts},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use brr_core::{ClientId, ClientKind, Error, Toolset};
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, JsonObject,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
        ToolAnnotations,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
    },
};
use schemars::{Schema, SchemaGenerator, generate::SchemaSettings};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    app::{App, Ctx},
    ops::{self, OpRegistration},
};

const SERVER_NAME: &str = "brr";
const CLIENT_ID_ARG: &str = "client_id";
const CLIENT_ID_HELP: &str = "from client_create; omit for a one-off call";
const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const INVALID_PARAMS: i32 = -32602;
// A stdio Client is the Forward backend's own; creating more from there is useless.
const CLIENT_CREATE_OP: &str = "client.create";
// Text of the limit error that `body::to_bytes` returns when the body is over the cap.
const LIMIT_ERROR: &str = "length limit exceeded";

mod forward;

pub use forward::{Ensure, Forwarder};

#[cfg(test)]
mod tests;

#[derive(Deserialize)]
struct ToolsetsQuery {
    toolsets: Option<String>,
}

pub enum Backend {
    Local(Arc<App>),
    Forward(Forwarder),
}

pub struct McpServer {
    backend: Backend,
    toolsets: Option<Vec<Toolset>>,
}

impl McpServer {
    pub fn new(backend: Backend, toolsets: Option<Vec<Toolset>>) -> McpServer {
        McpServer { backend, toolsets }
    }

    fn toolsets(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<Option<Vec<Toolset>>, ErrorData> {
        if self.toolsets.is_some() {
            return Ok(self.toolsets.clone());
        }

        let asked = match context.extensions.get::<Parts>() {
            Some(parts) => {
                let Query(query) = Query::<ToolsetsQuery>::try_from_uri(&parts.uri)
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;

                query
                    .toolsets
                    .as_deref()
                    .map(Toolset::parse_list)
                    .transpose()
                    .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?
            }
            None => None,
        };

        Ok(match (&self.backend, asked) {
            (_, Some(asked)) => Some(asked),
            (Backend::Local(app), None) => Some(app.settings().mcp.default_toolsets.clone()),
            // `brr mcp` resolves the setting itself and passes its list in.
            (Backend::Forward(_), None) => None,
        })
    }

    fn tool(&self, op: &OpRegistration) -> Option<Tool> {
        let spec = op.spec;
        let schemas = schemas(op)?;
        // An op with its own client_id (client.end, lease.list) keeps its meaning.
        let input = match self.backend {
            Backend::Local(_) if !schemas.declares_client_id => &schemas.input_with_client_id,
            _ => &schemas.input,
        };

        Some(
            Tool::new(spec.mcp_tool?, spec.summary, Arc::clone(input))
                .with_raw_output_schema(Arc::clone(&schemas.output))
                .annotate(ToolAnnotations::new().read_only(spec.read_only)),
        )
    }

    fn selected(&self, toolsets: Option<&Vec<Toolset>>, op: &OpRegistration) -> bool {
        let forwarded_create =
            matches!(self.backend, Backend::Forward(_)) && op.spec.name == CLIENT_CREATE_OP;

        !forwarded_create && toolsets.is_none_or(|toolsets| toolsets.contains(&op.spec.toolset))
    }

    async fn run(
        &self,
        op: &'static OpRegistration,
        mut input: JsonObject,
    ) -> Result<Value, Error> {
        let declares_client_id = schemas(op).is_some_and(|schemas| schemas.declares_client_id);
        let client_id = if declares_client_id {
            None
        } else {
            match input.remove(CLIENT_ID_ARG) {
                Some(Value::String(client_id)) => Some(ClientId::parse(&client_id)?),
                Some(_) => return Err(ops::invalid(CLIENT_ID_ARG, "must be a string")),
                None => None,
            }
        };
        let input = Value::Object(input);

        match &self.backend {
            Backend::Local(app) => match client_id {
                Some(client_id) => {
                    let client = app.clients.resolve(app, &client_id).await?;

                    ops::dispatch(
                        Ctx {
                            app: Arc::clone(app),
                            client,
                        },
                        op,
                        input,
                    )
                    .await
                }
                None => {
                    let guard = app.clients.implicit(app, ClientKind::Mcp);
                    let ctx = Ctx {
                        app: Arc::clone(app),
                        client: guard.client.clone(),
                    };

                    let result = ops::dispatch(ctx, op, input).await;
                    guard.finish().await;

                    result
                }
            },
            // brr mcp owns this Client and its Hold.
            Backend::Forward(forwarder) => forwarder.call(op.spec.name, &input).await,
        }
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        let mut config = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        config.server_info.name = SERVER_NAME.to_string();
        config.server_info.version = env!("CARGO_PKG_VERSION").to_string();

        config
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let toolsets = self.toolsets(&context)?;
        let tools = ops::all()
            .into_iter()
            .filter(|op| self.selected(toolsets.as_ref(), op))
            .filter_map(|op| self.tool(op))
            .collect();

        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let unknown =
            || ErrorData::invalid_params(format!("unknown tool `{}`", request.name), None);
        let op = ops::find_mcp(&request.name).ok_or_else(unknown)?;
        // A tool outside the selected toolsets is not listed, so it is not callable either.
        if !self.selected(self.toolsets(&context)?.as_ref(), op) {
            return Err(unknown());
        }

        let result = match self.run(op, request.arguments.unwrap_or_default()).await {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => {
                let body = error.body();
                let text = format!("{}: {}\n{}", body.code, body.message, json!(body));

                CallToolResult::error(vec![ContentBlock::text(text)])
            }
        };

        Ok(result.into())
    }
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .fallback_service(service(app))
        .layer(middleware::from_fn(screen))
}

// rmcp answers a body that is not valid JSON with 415 plain text, and a `tools/call` with the wrong
// shape as an unknown method (404); both are JSON-RPC errors.
async fn screen(request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = match body::to_bytes(body, crate::http::MAX_REQUEST_BYTES).await {
        Ok(bytes) => bytes,
        Err(error)
            if std::error::Error::source(&error).is_some_and(|e| e.to_string() == LIMIT_ERROR) =>
        {
            return rpc_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                &Value::Null,
                INVALID_REQUEST,
                "request body too large",
            );
        }
        Err(error) => {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                &Value::Null,
                INVALID_REQUEST,
                &format!("cannot read the request body: {error}"),
            );
        }
    };

    if parts.method == axum::http::Method::POST {
        let message = match serde_json::from_slice::<Value>(&bytes) {
            Ok(message) => message,
            Err(e) => {
                return rpc_error(
                    StatusCode::BAD_REQUEST,
                    &Value::Null,
                    PARSE_ERROR,
                    &format!("Parse error: {e}"),
                );
            }
        };

        if let Some(problem) = bad_tool_call(&message) {
            let id = message.get("id").unwrap_or(&Value::Null);

            return rpc_error(
                StatusCode::BAD_REQUEST,
                id,
                INVALID_PARAMS,
                &format!("Invalid params: {problem}"),
            );
        }
    }

    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}

fn bad_tool_call(message: &Value) -> Option<&'static str> {
    if message.get("method").and_then(Value::as_str) != Some("tools/call") {
        return None;
    }

    let Some(params) = message.get("params").and_then(Value::as_object) else {
        return Some("`params` must be an object");
    };
    if !params.get("name").is_some_and(Value::is_string) {
        return Some("`name` must be the tool name, a string");
    }

    match params.get("arguments") {
        None | Some(Value::Null | Value::Object(_)) => None,
        Some(_) => Some("`arguments` must be an object"),
    }
}

fn rpc_error(status: StatusCode, id: &Value, code: i32, message: &str) -> Response {
    let body = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } });

    (status, Json(body)).into_response()
}

fn service(app: Arc<App>) -> StreamableHttpService<McpServer, NeverSessionManager> {
    // ADR 0002: no Host or Origin checks, the daemon may bind a non-loopback address.
    let config = StreamableHttpServerConfig::default()
        .disable_allowed_hosts()
        .disable_allowed_origins()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(crate::http::MAX_REQUEST_BYTES);

    StreamableHttpService::new(
        move || Ok(McpServer::new(Backend::Local(Arc::clone(&app)), None)),
        Arc::new(NeverSessionManager::default()),
        config,
    )
}

pub fn inline_schema(build: fn(&mut SchemaGenerator) -> Schema) -> JsonObject {
    let mut generator = SchemaSettings::draft2020_12()
        .with(|settings| settings.inline_subschemas = true)
        .into_generator();

    match serde_json::to_value(build(&mut generator)) {
        Ok(Value::Object(schema)) => schema,
        _ => JsonObject::new(),
    }
}

struct Schemas {
    declares_client_id: bool,
    input: Arc<JsonObject>,
    input_with_client_id: Arc<JsonObject>,
    output: Arc<JsonObject>,
}

// Built once: a schema is a pure function of the op, and every call and listing needs it.
fn schemas(op: &OpRegistration) -> Option<&'static Schemas> {
    static SCHEMAS: OnceLock<HashMap<&'static str, Schemas>> = OnceLock::new();

    SCHEMAS
        .get_or_init(|| {
            ops::all()
                .into_iter()
                .map(|op| {
                    let spec = op.spec;
                    let declares_client_id = inline_schema(spec.input_schema)
                        .get("properties")
                        .is_some_and(|properties| properties.get(CLIENT_ID_ARG).is_some());
                    let schemas = Schemas {
                        declares_client_id,
                        input: object_schema(spec.input_schema, false),
                        input_with_client_id: object_schema(spec.input_schema, true),
                        output: object_schema(spec.output_schema, false),
                    };

                    (spec.name, schemas)
                })
                .collect()
        })
        .get(op.spec.name)
}

// MCP wants a self-contained object schema: subschemas inline, no $defs to dangle.
fn object_schema(
    build: fn(&mut SchemaGenerator) -> Schema,
    with_client_id: bool,
) -> Arc<JsonObject> {
    let mut schema = inline_schema(build);
    schema.insert("type".to_string(), json!("object"));

    if with_client_id
        && let Some(properties) = schema
            .entry("properties")
            .or_insert_with(|| json!({}))
            .as_object_mut()
    {
        properties.insert(
            CLIENT_ID_ARG.to_string(),
            json!({ "type": "string", "description": CLIENT_ID_HELP }),
        );
    }

    Arc::new(schema)
}
