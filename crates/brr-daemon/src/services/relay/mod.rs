use std::{
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use brr_core::{ConflictReason, Error, Instance, Tab};
use serde::Deserialize;
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::Notify,
    time,
};
use tracing::{debug, warn};

use crate::{app::App, services::instances::AttachSource};
use model::{Model, Out};

pub mod companion;
mod model;

#[cfg(test)]
mod tests;

const PROTOCOL: u32 = 1;
const INIT_TIMEOUT: Duration = Duration::from_secs(10);
const DUPLEX_BYTES: usize = 1024 * 1024;
const FRAME_DELIMITER: u8 = 0;
const PROTOCOL_ERROR: u16 = 1002;
const NORMAL: u16 = 1000;

#[derive(Default)]
pub struct Relay {
    unclaimed: Mutex<Vec<(Instance, Vec<Tab>)>>,
    arrived: Notify,
}

#[derive(Deserialize)]
struct Hello {
    protocol: u32,
    user_agent: String,
}

fn alive(app: &App, instance: &Instance) -> bool {
    app.instances
        .view(&instance.id)
        .is_some_and(|view| !view.instance.state.is_final())
}

impl Relay {
    // An extension connection always makes its own Live Instance (that is the user's "Allow and
    // connect"); this hands the newest one nobody has claimed to the caller, waiting for the user.
    pub async fn wait_attached(
        &self,
        app: &Arc<App>,
        timeout: Duration,
    ) -> Result<(Instance, Vec<Tab>), Error> {
        let wait = async {
            loop {
                let arrived = self.arrived.notified();
                tokio::pin!(arrived);
                arrived.as_mut().enable();

                if let Some(found) = self.take(app) {
                    return found;
                }
                arrived.await;
            }
        };

        time::timeout(timeout, wait)
            .await
            .map_err(|_| Error::Conflict {
                reason: ConflictReason::ExtensionUnanswered,
                detail: None,
            })
    }

    fn take(&self, app: &App) -> Option<(Instance, Vec<Tab>)> {
        let mut unclaimed = self
            .unclaimed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while let Some((instance, tabs)) = unclaimed.pop() {
            if alive(app, &instance) {
                return Some((instance, tabs));
            }
        }

        None
    }

    // Entries whose Instance has ended are dropped here, so connections nobody claims cannot pile up.
    fn publish(&self, app: &App, attached: (Instance, Vec<Tab>)) {
        let mut unclaimed = self
            .unclaimed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        unclaimed.retain(|(instance, _)| alive(app, instance));
        unclaimed.push(attached);
        drop(unclaimed);

        self.arrived.notify_waiters();
    }

    // One task per extension connection. Dropping the duplex when the socket ends is what tells
    // the Live driver the browser is gone.
    pub async fn serve(app: Arc<App>, mut socket: WebSocket) {
        let Some(mut model) = handshake(&mut socket).await else {
            return;
        };

        let (ours, theirs) = tokio::io::duplex(DUPLEX_BYTES);
        let attaching = Arc::clone(&app);
        tokio::spawn(async move {
            match attaching
                .instances
                .attach(&attaching, AttachSource::Extension(theirs))
                .await
            {
                Ok(attached) => attaching.relay.publish(&attaching, attached),
                Err(error) => warn!(%error, "extension attach failed"),
            }
        });

        let (reader, mut writer) = tokio::io::split(ours);
        let mut reader = BufReader::new(reader);
        // Kept across loop turns: read_until appends to it, so a read cancelled by the other
        // select arm loses nothing.
        let mut frame = Vec::new();

        'serving: loop {
            let outs = tokio::select! {
                message = socket.recv() => match message {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str::<Value>(&text) {
                        Ok(message) => model.on_extension(message),
                        Err(_) => continue,
                    },
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                },
                read = reader.read_until(FRAME_DELIMITER, &mut frame) => {
                    if !read.is_ok_and(|read| read > 0) {
                        break;
                    }
                    frame.pop();
                    let parsed = serde_json::from_slice::<Value>(&frame);
                    frame.clear();

                    match parsed {
                        Ok(frame) => model.on_cdp(frame),
                        Err(_) => continue,
                    }
                },
            };

            for out in outs {
                let sent = match out {
                    Out::Cdp(frame) => {
                        let mut bytes = frame.to_string().into_bytes();
                        bytes.push(FRAME_DELIMITER);

                        writer.write_all(&bytes).await.is_ok()
                    }
                    Out::Extension(command) => socket
                        .send(Message::Text(command.to_string().into()))
                        .await
                        .is_ok(),
                };
                if !sent {
                    break 'serving;
                }
            }
        }

        // A dropped half of a split stream does not signal EOF; the driver learns the browser is
        // gone from this.
        let _ = writer.shutdown().await;
        close(&mut socket, NORMAL, "relay closed").await;
        debug!("extension relay connection ended");
    }
}

async fn close(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

fn text_of(message: Option<Result<Message, axum::Error>>) -> Option<Value> {
    match message? {
        Ok(Message::Text(text)) => serde_json::from_str(&text).ok(),
        _ => None,
    }
}

// hello, then the tabs the user shares, then initialized: browser-level CDP only starts once
// the relay knows the tabs.
async fn handshake(socket: &mut WebSocket) -> Option<Model> {
    let hello = time::timeout(INIT_TIMEOUT, socket.recv()).await.ok()?;
    let hello = text_of(hello)
        .filter(|message| message["method"] == "extension.hello")
        .and_then(|message| serde_json::from_value::<Hello>(message["params"][0].clone()).ok());
    let Some(hello) = hello.filter(|hello| hello.protocol == PROTOCOL) else {
        close(socket, PROTOCOL_ERROR, "unsupported companion protocol").await;

        return None;
    };

    let mut model = Model::new(hello.user_agent);
    let collected = time::timeout(INIT_TIMEOUT, async {
        loop {
            let message = text_of(socket.recv().await);
            let Some(message) = message else {
                return false;
            };
            if message["method"] == "extension.initialized" {
                return true;
            }
            model.on_extension(message);
        }
    })
    .await;
    if collected != Ok(true) {
        close(
            socket,
            PROTOCOL_ERROR,
            "the companion did not finish initializing",
        )
        .await;

        return None;
    }

    Some(model)
}
