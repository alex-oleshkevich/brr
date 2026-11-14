use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use axum::extract::ws::Utf8Bytes;
use brr_core::{ClientId, Error, InstanceId, Size, TabId};
use brr_engine::page::{
    Dialog, DialogType, FrameMetadata, KeyInput, KeyKind, MouseButton, MouseInput, MouseKind, Page,
    Screencast, ScreencastFrame,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{
    sync::{Mutex as AsyncMutex, broadcast, watch},
    task::JoinHandle,
};

use crate::{app::App, ops::invalid, services::events::Published};

#[cfg(test)]
mod tests;

pub const SCREENCAST_MAX: Size = Size {
    width: 1920,
    height: 1080,
};
pub const SCREENCAST_QUALITY: u8 = 70;

const TAB_CLOSED: &str = "tab.closed";

// A viewer further behind than this skips to the newest message.
const VIEWER_BUFFER: usize = 4;

#[derive(Clone)]
pub enum Update {
    Message(Utf8Bytes),
    Closed,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseEvent {
    Down,
    Up,
    Move,
    Wheel,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Left,
    Middle,
    Right,
    #[default]
    None,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyEvent {
    Down,
    Up,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Mouse {
        event: MouseEvent,
        x: f64,
        y: f64,
        #[serde(default)]
        button: Button,
        #[serde(default)]
        click_count: u32,
        #[serde(default)]
        modifiers: u32,
        #[serde(default)]
        delta_x: f64,
        #[serde(default)]
        delta_y: f64,
    },
    Key {
        event: KeyEvent,
        #[serde(default)]
        key: String,
        #[serde(default)]
        code: String,
        #[serde(default)]
        key_code: u32,
        #[serde(default)]
        text: String,
        #[serde(default)]
        modifiers: u32,
    },
    Text {
        text: String,
    },
    DialogAnswer {
        accept: bool,
        prompt_text: Option<String>,
    },
}

#[derive(Serialize)]
struct FrameMessage<'a> {
    r#type: &'static str,
    data: &'a str,
    metadata: MetadataMessage,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetadataMessage {
    device_width: f64,
    device_height: f64,
    page_scale_factor: f64,
    offset_top: f64,
}

// What input needs from the frame the user saw: its pixel size and the page geometry it carried.
#[derive(Clone, Copy)]
struct Geometry {
    size: (u32, u32),
    metadata: FrameMetadata,
}

#[derive(Default)]
struct Latest {
    frame: Option<Utf8Bytes>,
    dialog: Option<Utf8Bytes>,
    geometry: Option<Geometry>,
}

struct Shared {
    updates: broadcast::Sender<Update>,
    latest: Mutex<Latest>,
    page: Page,
}

struct Entry {
    shared: Arc<Shared>,
    viewers: usize,
    producer: JoinHandle<()>,
}

#[derive(Default)]
struct Producers {
    live: HashMap<TabId, Entry>,
    // Aborted producers whose Screencast may not be dropped yet: the next start waits for them,
    // or the engine would still answer "already running".
    ending: HashMap<TabId, JoinHandle<()>>,
}

impl Producers {
    // A closed Tab is never viewed again, so no later start would take its handle.
    fn drop_finished(&mut self) {
        self.ending.retain(|_, ended| !ended.is_finished());
    }
}

type Tabs = Mutex<Producers>;

#[derive(Default)]
pub struct Screencasts {
    tabs: Arc<Tabs>,
    // Serializes the start of one Tab's producer, so two first viewers share one; other Tabs
    // are not held up by a slow browser.
    starting: Mutex<HashMap<TabId, Arc<AsyncMutex<()>>>>,
}

// One viewer of one Tab; leaving the last one stops the screencast.
pub struct Viewer {
    pub updates: broadcast::Receiver<Update>,
    shared: Arc<Shared>,
    tabs: Arc<Tabs>,
    instance_id: InstanceId,
    tab_id: TabId,
    client_id: ClientId,
}

impl Screencasts {
    pub async fn subscribe(
        &self,
        app: &Arc<App>,
        client_id: &ClientId,
        instance_id: InstanceId,
        tab_id: TabId,
    ) -> Result<Viewer, Error> {
        let lock = self.start_lock(tab_id);
        let started = {
            let _starting = lock.lock().await;

            match self.join(&tab_id) {
                Some(shared) => Ok(shared),
                None => self.start(app, &instance_id, tab_id).await,
            }
        };
        self.drop_start_lock(&tab_id, lock);

        let shared = started?;

        Ok(Viewer {
            updates: shared.updates.subscribe(),
            shared,
            tabs: Arc::clone(&self.tabs),
            instance_id,
            tab_id,
            client_id: client_id.clone(),
        })
    }

    fn start_lock(&self, tab_id: TabId) -> Arc<AsyncMutex<()>> {
        let mut locks = self.starting.lock().unwrap_or_else(PoisonError::into_inner);

        Arc::clone(locks.entry(tab_id).or_default())
    }

    // The map and the caller hold it; anyone else is a start that is still waiting for it.
    fn drop_start_lock(&self, tab_id: &TabId, lock: Arc<AsyncMutex<()>>) {
        let mut locks = self.starting.lock().unwrap_or_else(PoisonError::into_inner);
        if Arc::strong_count(&lock) == 2 {
            locks.remove(tab_id);
        }
    }

    fn join(&self, tab_id: &TabId) -> Option<Arc<Shared>> {
        let mut tabs = self.tabs.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = tabs.live.get_mut(tab_id)?;
        entry.viewers += 1;

        Some(Arc::clone(&entry.shared))
    }

    async fn start(
        &self,
        app: &Arc<App>,
        instance_id: &InstanceId,
        tab_id: TabId,
    ) -> Result<Arc<Shared>, Error> {
        let previous = self
            .tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .ending
            .remove(&tab_id);
        if let Some(previous) = previous {
            // Aborted: it only has its locals left to drop.
            let _ = previous.await;
        }

        // Before the screencast starts, so a Tab that closes meanwhile is not missed.
        let closed = app.events.subscribe();
        let page = app.instances.page(instance_id, &tab_id).await?;
        let screencast = page
            .start_screencast(SCREENCAST_MAX, SCREENCAST_QUALITY)
            .await?;
        let dialog = page.state().dialog();
        let shared = Arc::new(Shared {
            updates: broadcast::channel(VIEWER_BUFFER).0,
            latest: Mutex::default(),
            page,
        });

        // Under the lock, so a producer that ends at once finds its entry to remove.
        let mut tabs = self.tabs.lock().unwrap_or_else(PoisonError::into_inner);
        let producer = tokio::spawn(produce(
            Arc::clone(&shared),
            screencast,
            dialog,
            closed,
            Arc::clone(&self.tabs),
            tab_id,
        ));
        tabs.live.insert(
            tab_id,
            Entry {
                shared: Arc::clone(&shared),
                viewers: 1,
                producer,
            },
        );

        Ok(shared)
    }
}

// Ends when the Tab closes, whatever the reason (an Instance that stops or crashes closes its
// Tabs too); aborting it (the last viewer left) drops the Screencast, which stops the browser's.
async fn produce(
    shared: Arc<Shared>,
    mut screencast: Screencast,
    mut dialog: watch::Receiver<Option<Dialog>>,
    mut closed: broadcast::Receiver<Arc<Published>>,
    tabs: Arc<Tabs>,
    tab_id: TabId,
) {
    // What a viewer that joins now needs; nobody is listening yet, so nothing is broadcast.
    shared
        .latest
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .dialog = dialog_message(dialog.borrow_and_update().as_ref());

    let tab_text = tab_id.to_string();
    loop {
        tokio::select! {
            frame = screencast.next() => match frame {
                Some(frame) => publish_frame(&shared, &frame),
                None => break,
            },
            changed = dialog.changed() => {
                if changed.is_err() {
                    break;
                }
                publish_dialog(&shared, dialog.borrow_and_update().as_ref());
            }
            event = closed.recv() => match event {
                Ok(published) => {
                    if published.event.kind == TAB_CLOSED && published.event.data["tab_id"] == tab_text {
                        break;
                    }
                }
                // The bus is gone, or it dropped events among which may be this Tab's closing:
                // ending makes the viewers reconnect, which finds the Tab or its absence.
                Err(_) => break,
            },
        }
    }

    // Out of the map first: a viewer that arrives now starts a fresh producer and sees why
    // that fails.
    let mut tabs = tabs.lock().unwrap_or_else(PoisonError::into_inner);
    if tabs
        .live
        .get(&tab_id)
        .is_some_and(|entry| Arc::ptr_eq(&entry.shared, &shared))
    {
        tabs.live.remove(&tab_id);
    }
    let _ = shared.updates.send(Update::Closed);
}

// The frame is serialized once; every viewer shares the bytes.
fn publish_frame(shared: &Shared, frame: &ScreencastFrame) {
    let metadata = frame.metadata;
    let message = FrameMessage {
        r#type: "frame",
        data: &frame.data,
        metadata: MetadataMessage {
            device_width: metadata.device_width,
            device_height: metadata.device_height,
            page_scale_factor: metadata.page_scale_factor,
            offset_top: metadata.offset_top,
        },
    };
    let Ok(json) = serde_json::to_string(&message) else {
        return;
    };
    let json = Utf8Bytes::from(json);

    {
        let mut latest = shared.latest.lock().unwrap_or_else(PoisonError::into_inner);
        latest.frame = Some(json.clone());
        latest.geometry = Some(Geometry {
            size: (frame.width, frame.height),
            metadata,
        });
    }
    let _ = shared.updates.send(Update::Message(json));
}

const DIALOG_CLOSED: &str = r#"{"type":"dialog_closed"}"#;

fn dialog_message(dialog: Option<&Dialog>) -> Option<Utf8Bytes> {
    dialog.map(|dialog| {
        let dialog_type = match dialog.dialog_type {
            DialogType::Alert => "alert",
            DialogType::Confirm => "confirm",
            DialogType::Prompt => "prompt",
            DialogType::Beforeunload => "beforeunload",
        };

        Utf8Bytes::from(
            json!({
                "type": "dialog",
                "dialog_type": dialog_type,
                "message": dialog.message,
                "default_prompt": dialog.default_prompt,
            })
            .to_string(),
        )
    })
}

fn publish_dialog(shared: &Shared, dialog: Option<&Dialog>) {
    let message = dialog_message(dialog);

    shared
        .latest
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .dialog
        .clone_from(&message);
    let _ = shared.updates.send(Update::Message(
        message.unwrap_or_else(|| Utf8Bytes::from_static(DIALOG_CLOSED)),
    ));
}

impl Viewer {
    // What a viewer that has just joined starts from: the open dialog, then the newest frame.
    pub fn snapshot(&self) -> Vec<Utf8Bytes> {
        let latest = self
            .shared
            .latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        latest.dialog.iter().chain(&latest.frame).cloned().collect()
    }

    // For a viewer that lost messages: the closing of a dialog may be among them. Frames need
    // no repair, since the ones still buffered are the newest.
    pub fn dialog_state(&self) -> Utf8Bytes {
        let latest = self
            .shared
            .latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        latest
            .dialog
            .clone()
            .unwrap_or_else(|| Utf8Bytes::from_static(DIALOG_CLOSED))
    }

    // LC6: input needs the Tab awaiting a person, or the Lease (after a take over).
    pub async fn input(&self, app: &Arc<App>, message: ClientMessage) -> Result<(), Error> {
        self.permit(app).await?;
        app.instances
            .activity(&self.instance_id, Some(&self.tab_id));

        let page = &self.shared.page;
        match message {
            ClientMessage::Mouse {
                event,
                x,
                y,
                button,
                click_count,
                modifiers,
                delta_x,
                delta_y,
            } => {
                let geometry = self
                    .shared
                    .latest
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .geometry;
                // No frame yet: there is nothing the user saw to point at.
                let Some(geometry) = geometry else {
                    return Ok(());
                };
                let (x, y) = to_css(x, y, geometry.size, &geometry.metadata);

                page.mouse(&MouseInput {
                    kind: match event {
                        MouseEvent::Down => MouseKind::Down,
                        MouseEvent::Up => MouseKind::Up,
                        MouseEvent::Move => MouseKind::Move,
                        MouseEvent::Wheel => MouseKind::Wheel,
                    },
                    x,
                    y,
                    button: match button {
                        Button::Left => MouseButton::Left,
                        Button::Middle => MouseButton::Middle,
                        Button::Right => MouseButton::Right,
                        Button::None => MouseButton::None,
                    },
                    click_count,
                    modifiers,
                    delta_x,
                    delta_y,
                })
                .await
            }
            ClientMessage::Key {
                event,
                key,
                code,
                key_code,
                text,
                modifiers,
            } => {
                page.key(&KeyInput {
                    kind: match event {
                        KeyEvent::Down => KeyKind::Down,
                        KeyEvent::Up => KeyKind::Up,
                    },
                    key,
                    code,
                    key_code,
                    text,
                    modifiers,
                })
                .await
            }
            ClientMessage::Text { text } => page.insert_text(&text).await,
            ClientMessage::DialogAnswer {
                accept,
                prompt_text,
            } => page.handle_dialog(accept, prompt_text.as_deref()).await,
        }
    }

    async fn permit(&self, app: &Arc<App>) -> Result<(), Error> {
        if app
            .instances
            .tab_awaits_human(&self.instance_id, &self.tab_id)
        {
            return Ok(());
        }

        match app.leases.tab_lease(&self.tab_id).await {
            Some(lease) if lease.client_id == self.client_id => Ok(()),
            Some(lease) => Err(Error::LeaseHeld {
                client_id: lease.client_id,
            }),
            None => Err(invalid(
                "input",
                "take the Tab over before sending input to it",
            )),
        }
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        let mut tabs = self.tabs.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(entry) = tabs.live.get_mut(&self.tab_id) else {
            return;
        };
        if !Arc::ptr_eq(&entry.shared, &self.shared) {
            return;
        }

        entry.viewers -= 1;
        if entry.viewers == 0
            && let Some(entry) = tabs.live.remove(&self.tab_id)
        {
            entry.producer.abort();
            tabs.drop_finished();
            tabs.ending.insert(self.tab_id, entry.producer);
        }
    }
}

// Frame pixels to CSS pixels: the frame is the page scaled by device size over frame size, and
// by the page's own zoom on top.
pub fn to_css(x: f64, y: f64, frame: (u32, u32), metadata: &FrameMetadata) -> (f64, f64) {
    let zoom = if metadata.page_scale_factor > 0.0 {
        metadata.page_scale_factor
    } else {
        1.0
    };

    (
        x * metadata.device_width / f64::from(frame.0) / zoom,
        y * metadata.device_height / f64::from(frame.1) / zoom,
    )
}
