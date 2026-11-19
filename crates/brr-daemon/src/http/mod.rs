use std::sync::Arc;

use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    http::Uri,
    response::{IntoResponse, Response},
    routing::get,
};
use brr_core::Error;

use crate::{app::App, mcp};

mod artifacts;
mod cdp;
mod metrics;
mod oauth;
mod oneshot;
mod openapi;
mod ops;
mod presence;
mod profile_archive;
mod relay;
mod screencast;
mod sse;
mod ui;

pub use openapi::openapi;

const API_PREFIX: &str = "/v1/";
// 48 MiB of upload as base64 (64 MiB) plus the JSON envelope.
pub const MAX_REQUEST_BYTES: usize = 66 * 1024 * 1024;

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .merge(ops::routes())
        .merge(artifacts::routes())
        .merge(oneshot::routes())
        .merge(cdp::routes())
        .merge(relay::routes())
        .merge(screencast::routes())
        .merge(profile_archive::routes())
        .merge(oauth::routes())
        .route("/openapi.json", get(|| async { Json(openapi()) }))
        .route("/metrics", get(metrics::serve))
        .route("/v1/events/stream", get(sse::stream))
        .route("/v1/clients/{client_id}/presence", get(presence::upgrade))
        .nest_service("/mcp", mcp::router(app.clone()))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .fallback(fallback)
        .method_not_allowed_fallback(not_found)
        .with_state(app)
}

async fn fallback(uri: Uri) -> Response {
    if uri.path().starts_with(API_PREFIX) {
        return not_found().await.into_response();
    }

    ui::serve(uri.path())
}

// A wrong method on a known path is also a route that does not exist; §14 has no 405.
async fn not_found() -> ops::ApiError {
    ops::ApiError(Error::NotFound {
        what: "route".to_string(),
    })
}
