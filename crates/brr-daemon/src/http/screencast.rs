use std::sync::Arc;

use axum::{
    Router,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        rejection::QueryRejection,
        ws::{CloseFrame, Message, Utf8Bytes, WebSocket, rejection::WebSocketUpgradeRejection},
    },
    response::Response,
    routing::get,
};
use brr_core::{Capability, ClientId, Error, InstanceId, TabId, TabState};
use brr_store::{build, instance, tab};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use super::ops::ApiError;
use crate::{
    app::App,
    ops::invalid,
    services::{
        clients::Hold,
        screencast::{ClientMessage, Update, Viewer},
    },
    wire::{NORMAL_CLOSE, websocket_only},
};

// Input is a small JSON object; the largest is pasted text.
const MAX_INPUT_BYTES: usize = 64 * 1024;

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/v1/tabs/{tab_id}/screencast", get(upgrade))
}

// Browsers cannot set headers on a WebSocket, so the Client rides in the query.
#[derive(Deserialize)]
struct ScreencastQuery {
    client_id: String,
}

// What can fail without a socket does so before the upgrade, as a normal error response. A Build
// without screencast is the exception: the UI shows the message the socket carries.
async fn upgrade(
    State(app): State<Arc<App>>,
    Path(tab_id): Path<String>,
    query: Result<Query<ScreencastQuery>, QueryRejection>,
    socket: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, ApiError> {
    let socket = socket.map_err(|_| websocket_only())?;
    let Query(query) = query.map_err(|rejection| Error::InvalidArgument {
        field: "query".to_string(),
        message: rejection.body_text(),
    })?;
    let tab_id: TabId = tab_id
        .parse()
        .map_err(|_| invalid("tab_id", "use the id of a tab"))?;

    let client = app
        .clients
        .resolve(&app, &ClientId::parse(&query.client_id)?)
        .await?;
    let stored = app.store.run(move |c| tab::get(c, &tab_id)).await?;
    if stored.state == TabState::Closed {
        return Err(Error::NotFound {
            what: format!("open tab {tab_id}"),
        }
        .into());
    }

    let instance_id = stored.instance_id;
    let capabilities = app
        .store
        .run(move |c| match instance::get(c, &instance_id)?.build_id {
            Some(build_id) => Ok(build::get(c, &build_id)?.capabilities),
            None => Ok(Vec::new()),
        })
        .await?;
    let hold = app.clients.hold(&app, &client.client_id).await?;

    Ok(socket
        .max_message_size(MAX_INPUT_BYTES)
        .max_frame_size(MAX_INPUT_BYTES)
        .on_upgrade(move |socket| {
            let supported = capabilities.contains(&Capability::Screencast);

            run(
                socket,
                app,
                client.client_id,
                instance_id,
                tab_id,
                supported,
                hold,
            )
        }))
}

async fn run(
    mut socket: WebSocket,
    app: Arc<App>,
    client_id: ClientId,
    instance_id: InstanceId,
    tab_id: TabId,
    supported: bool,
    // Held for the socket's life, so the Client outlives its live view.
    _hold: Hold,
) {
    let viewer = if supported {
        app.screencasts
            .subscribe(&app, &client_id, instance_id, tab_id)
            .await
    } else {
        Err(Error::EngineUnsupported {
            capability: Capability::Screencast,
        })
    };
    match viewer {
        Ok(viewer) => serve(&mut socket, &app, viewer).await,
        Err(error) => {
            send_error(&mut socket, &error).await;
        }
    }

    let frame = CloseFrame {
        code: NORMAL_CLOSE,
        reason: "".into(),
    };
    let _ = socket.send(Message::Close(Some(frame))).await;
}

async fn serve(socket: &mut WebSocket, app: &Arc<App>, mut viewer: Viewer) {
    for message in viewer.snapshot() {
        if !send(socket, message).await {
            return;
        }
    }

    loop {
        tokio::select! {
            update = viewer.updates.recv() => match update {
                Ok(Update::Message(message)) => {
                    if !send(socket, message).await {
                        return;
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    if !send(socket, viewer.dialog_state()).await {
                        return;
                    }
                }
                Ok(Update::Closed) | Err(RecvError::Closed) => {
                    send(socket, Utf8Bytes::from_static(r#"{"type":"closed"}"#)).await;

                    return;
                }
            },
            received = socket.recv() => match received {
                Some(Ok(Message::Text(text))) => {
                    // One input at a time, in order: the next waits for this one's answer.
                    let handled = match serde_json::from_str::<ClientMessage>(&text) {
                        Ok(message) => viewer.input(app, message).await,
                        Err(error) => Err(invalid("message", &error.to_string())),
                    };
                    if let Err(error) = handled
                        && !send_error(socket, &error).await
                    {
                        return;
                    }
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

async fn send(socket: &mut WebSocket, message: Utf8Bytes) -> bool {
    socket.send(Message::Text(message)).await.is_ok()
}

async fn send_error(socket: &mut WebSocket, error: &Error) -> bool {
    let message = json!({"type": "error", "code": error.code(), "message": error.to_string()});

    send(socket, Utf8Bytes::from(message.to_string())).await
}
