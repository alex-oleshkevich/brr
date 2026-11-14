use std::sync::Arc;

use axum::{
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{WebSocket, rejection::WebSocketUpgradeRejection},
    },
    response::Response,
};
use brr_core::ClientId;

use super::ops::ApiError;
use crate::{app::App, services::clients::Hold, wire::websocket_only};

pub async fn upgrade(
    State(app): State<Arc<App>>,
    Path(client_id): Path<String>,
    socket: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let socket = socket.map_err(|_| websocket_only())?;
    let hold = app
        .clients
        .hold(&app, &ClientId::parse(&client_id)?)
        .await?;

    Ok(socket.on_upgrade(|socket| open(socket, hold)))
}

// The Hold lives as long as the socket; dropping it lets cli and mcp Clients end at once.
async fn open(mut socket: WebSocket, hold: Hold) {
    while let Some(Ok(_)) = socket.recv().await {}

    drop(hold);
}
