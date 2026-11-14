use std::sync::Arc;

use axum::{
    Router,
    extract::{State, WebSocketUpgrade, ws::rejection::WebSocketUpgradeRejection},
    http::{HeaderMap, StatusCode, header::ORIGIN},
    response::{IntoResponse, Response},
    routing::get,
};
use brr_core::Error;
use brr_proto::MAX_MESSAGE_BYTES;

use super::ops::ApiError;
use crate::{app::App, services::relay::Relay};

const EXTENSION_ORIGINS: [&str; 2] = ["chrome-extension://", "moz-extension://"];

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/v1/extension-relay", get(upgrade))
}

// The one Origin check in the daemon (ADR 0002, Exception): a web page can open this socket too,
// but it cannot forge an extension Origin, so only the companion becomes a browser to the agent.
fn from_extension(headers: &HeaderMap) -> bool {
    headers
        .get(ORIGIN)
        .and_then(|origin| origin.to_str().ok())
        .is_some_and(|origin| {
            EXTENSION_ORIGINS
                .iter()
                .any(|prefix| origin.starts_with(prefix))
        })
}

async fn upgrade(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    socket: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    if !from_extension(&headers) {
        return Ok((
            StatusCode::FORBIDDEN,
            "only the brr companion extension may connect to the relay",
        )
            .into_response());
    }
    let socket = socket.map_err(|_| Error::InvalidArgument {
        field: "upgrade".to_string(),
        message: "this route speaks WebSocket; the brr companion extension connects here"
            .to_string(),
    })?;

    Ok(socket
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| Relay::serve(app, socket)))
}
