use std::sync::Arc;

use axum::{
    Json, Router,
    extract::Query,
    http::{
        HeaderMap, StatusCode,
        header::{CACHE_CONTROL, HOST},
    },
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use ulid::Ulid;

use crate::app::App;

const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";
const PROTECTED_RESOURCE_PATH: &str = "/.well-known/oauth-protected-resource";
const PROTECTED_MCP_PATH: &str = "/.well-known/oauth-protected-resource/mcp";
const AUTHORIZE_PATH: &str = "/oauth/authorize";
const TOKEN_PATH: &str = "/oauth/token";
const REGISTER_PATH: &str = "/oauth/register";
const MCP_PATH: &str = "/mcp";
const FORWARDED_PROTO: &str = "x-forwarded-proto";
const DEFAULT_PROTO: &str = "http";
const NO_STORE: &str = "no-store";
// A form without `action` posts back to this URL, query string included, so the page embeds
// nothing from the request.
const ALLOW_PAGE: &str = include_str!("allow.html");

#[derive(Deserialize)]
struct Authorize {
    redirect_uri: String,
    state: Option<String>,
}

// No surface checks credentials (ADR 0002): this only lets agents that insist on OAuth connect.
pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route(AUTHORIZATION_SERVER_PATH, get(authorization_server))
        .route(PROTECTED_RESOURCE_PATH, get(protected_resource))
        .route(PROTECTED_MCP_PATH, get(protected_resource))
        .route(AUTHORIZE_PATH, get(Html(ALLOW_PAGE)).post(allow))
        .route(TOKEN_PATH, post(token))
        .route(REGISTER_PATH, post(register))
}

fn origin(headers: &HeaderMap) -> String {
    let header = |name| headers.get(name).and_then(|value| value.to_str().ok());
    let proto = header(FORWARDED_PROTO).unwrap_or(DEFAULT_PROTO);
    let host = header(HOST.as_str()).unwrap_or_default();

    format!("{proto}://{host}")
}

async fn authorization_server(headers: HeaderMap) -> Json<Value> {
    let origin = origin(&headers);

    Json(json!({
        "issuer": origin,
        "authorization_endpoint": format!("{origin}{AUTHORIZE_PATH}"),
        "token_endpoint": format!("{origin}{TOKEN_PATH}"),
        "registration_endpoint": format!("{origin}{REGISTER_PATH}"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
    }))
}

async fn protected_resource(headers: HeaderMap) -> Json<Value> {
    let origin = origin(&headers);

    Json(json!({
        "resource": format!("{origin}{MCP_PATH}"),
        "authorization_servers": [origin],
    }))
}

async fn allow(Query(request): Query<Authorize>) -> Response {
    let mut target = match Url::parse(&request.redirect_uri) {
        Ok(target) => target,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("redirect_uri is not a URL: {error}"),
            )
                .into_response();
        }
    };

    let mut query = target.query_pairs_mut();
    query.append_pair("code", &Ulid::generate().to_string());
    if let Some(state) = &request.state {
        query.append_pair("state", state);
    }
    drop(query);

    Redirect::to(target.as_str()).into_response()
}

async fn token() -> Response {
    let tokens = json!({
        "access_token": Ulid::generate().to_string(),
        "refresh_token": Ulid::generate().to_string(),
        "token_type": "Bearer",
    });

    ([(CACHE_CONTROL, NO_STORE)], Json(tokens)).into_response()
}

async fn register(Json(mut client): Json<Map<String, Value>>) -> (StatusCode, Json<Value>) {
    client.insert("client_id".to_string(), json!(Ulid::generate().to_string()));

    (StatusCode::CREATED, Json(Value::Object(client)))
}
