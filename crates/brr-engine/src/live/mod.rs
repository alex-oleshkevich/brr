use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use brr_cdp::{CdpEvent, SessionId};
use brr_core::{Error, InstanceId, TargetId, WaitUntil};
use serde_json::json;
use tokio::io::DuplexStream;
use tokio::sync::{Notify, OnceCell, mpsc};
use tracing::debug;

use crate::chromium::is_user_page;
use crate::common::{EVENTS_QUEUE, internal};
use crate::{Chromium, Driver, DriverEvent, Navigated};

#[cfg(test)]
mod tests;

// Shared by the Live driver and its target task.
#[derive(Default)]
pub(crate) struct Shared {
    // One attach per target, however many first calls race.
    cells: Mutex<HashMap<TargetId, Arc<OnceCell<()>>>>,
    // Pages brr created, so brr may close them.
    opened: Mutex<HashSet<TargetId>>,
    // Held while brr creates a page: the target task announces that page only after it was
    // recorded in `opened`, so a Tab never shows up as the user's first.
    creating: tokio::sync::Mutex<()>,
}

impl Shared {
    pub(crate) async fn creating(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.creating.lock().await
    }

    pub(crate) fn is_opened(&self, target_id: &TargetId) -> bool {
        self.opened
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(target_id)
    }

    pub(crate) fn forget(&self, target_id: &TargetId) {
        self.cells
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(target_id);
        self.opened
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(target_id);
    }
}

const PORT_FILE: &str = "DevToolsActivePort";
const INSTALL_NAME_FIELD: &str = "install_name";
const DISCOVERY_QUIET: Duration = Duration::from_millis(250);
const DISCOVERY_LIMIT: Duration = Duration::from_secs(2);

// Install name -> directory under the XDG config root.
pub const INSTALL_NAMES: &[(&str, &str)] = &[
    ("chrome", "google-chrome"),
    ("chrome-beta", "google-chrome-beta"),
    ("chromium", "chromium"),
    ("brave", "BraveSoftware/Brave-Browser"),
    ("edge", "microsoft-edge"),
];

pub enum Connection {
    WebSocket(String),
    Relay(DuplexStream),
}

pub struct AttachSpec {
    pub instance_id: InstanceId,
    pub connection: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTarget {
    pub target_id: TargetId,
    pub url: String,
    pub title: String,
}

// `events` is bounded like `Launched::events`: drain it in its own task.
pub struct Attached {
    pub driver: Driver,
    pub events: mpsc::Receiver<DriverEvent>,
    pub browser_version: String,
    // The pages that existed when discovery finished. `events` also announces each of them with
    // TabCreated, TabAttached, TabLoaded and TabInfo, like a page that opens later.
    pub targets: Vec<LiveTarget>,
}

// A browser brr did not start and never kills. Its pages are discovered but attached one by one,
// on first use: attaching shows the user's debugging banner and counts as a client on the page.
#[derive(Clone)]
pub struct Live {
    pub(crate) chromium: Chromium,
    shared: Arc<Shared>,
    stop: Arc<Notify>,
}

// `config_root` is $XDG_CONFIG_HOME, the same lookup Chromium itself makes.
pub fn devtools_active_port(config_root: &Path, install_name: &str) -> Result<String, Error> {
    let (_, dir) = INSTALL_NAMES
        .iter()
        .find(|(name, _)| *name == install_name)
        .ok_or_else(|| Error::InvalidArgument {
            field: INSTALL_NAME_FIELD.to_owned(),
            message: format!(
                "expected one of {}",
                INSTALL_NAMES
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        })?;

    let text = std::fs::read_to_string(config_root.join(dir).join(PORT_FILE)).map_err(|_| {
        Error::DependencyMissing {
            name: PORT_FILE.to_owned(),
            help: format!(
                "enable remote debugging at chrome://inspect/#remote-debugging in {install_name}, \
                 then attach again"
            ),
        }
    })?;
    let mut lines = text.lines();
    let (Some(port), Some(path)) = (lines.next(), lines.next()) else {
        return Err(internal(format!("{PORT_FILE} is malformed")));
    };

    Ok(format!("ws://127.0.0.1:{port}{path}"))
}

impl Driver {
    // Sends Target.setDiscoverTargets only: never setAutoAttach or waitForDebuggerOnStart, which
    // would pause every tab the user opens until brr resumes it.
    pub async fn attach(spec: AttachSpec) -> Result<Attached, Error> {
        let (cdp, mut cdp_events) = match spec.connection {
            Connection::WebSocket(url) => brr_cdp::connect_ws(&url).await?,
            Connection::Relay(stream) => brr_cdp::start_duplex(stream),
        };
        let browser_version = cdp.browser_version().await?;
        cdp.set_discover_targets().await?;

        // Chromium reports every existing target right after the call, so a quiet gap ends it.
        let mut targets: Vec<LiveTarget> = Vec::new();
        let deadline = Instant::now() + DISCOVERY_LIMIT;
        loop {
            let wait = DISCOVERY_QUIET.min(deadline.saturating_duration_since(Instant::now()));
            let Ok(Some(event)) = tokio::time::timeout(wait, cdp_events.recv()).await else {
                break;
            };

            match event {
                CdpEvent::TargetCreated(info) if is_user_page(&info) => targets.push(LiveTarget {
                    target_id: info.target_id,
                    url: info.url,
                    title: info.title,
                }),
                CdpEvent::TargetInfoChanged(info) => {
                    if let Some(target) = targets
                        .iter_mut()
                        .find(|target| target.target_id == info.target_id)
                    {
                        target.url = info.url;
                        target.title = info.title;
                    }
                }
                CdpEvent::TargetDestroyed { target_id } => {
                    targets.retain(|target| target.target_id != target_id);
                }
                _ => {}
            }
        }

        let (events_tx, events) = mpsc::channel(EVENTS_QUEUE);
        let stop = Arc::new(Notify::new());
        let shared = Arc::new(Shared::default());
        let chromium = Chromium::connect(
            cdp,
            cdp_events,
            events_tx,
            stop.clone(),
            targets.clone(),
            shared.clone(),
        );
        debug!(instance_id = %spec.instance_id, pages = targets.len(), "attached to a live browser");

        Ok(Attached {
            driver: Driver::Live(Live {
                chromium,
                shared,
                stop,
            }),
            events,
            browser_version,
            targets,
        })
    }

    // Ends brr's use of a Live browser: the connection closes, whoever still holds the Driver or
    // a Page. The browser and its pages stay as they were.
    pub async fn detach(&self) -> Result<(), Error> {
        let Driver::Live(live) = self else {
            return Err(internal("only a Live browser detaches; use close"));
        };
        live.detach().await;

        Ok(())
    }

    pub fn is_live(&self) -> bool {
        matches!(self, Driver::Live(_))
    }

    // Whether brr made this page. Everything a launched browser shows is brr's; in a Live browser
    // only the pages `open_tab` created are, and brr closes no others.
    pub fn opened_by_brr(&self, target_id: &TargetId) -> bool {
        match self {
            Driver::Live(live) => live.shared.is_opened(target_id),
            _ => true,
        }
    }

    // Makes the Tab usable for `Driver::page`; a no-op for browsers that attach on their own.
    // Concurrent first calls share one attach.
    pub async fn attach_tab(&self, target_id: &TargetId) -> Result<(), Error> {
        match self {
            Driver::Live(live) => live.attach_tab(target_id).await,
            _ => Ok(()),
        }
    }
}

impl Live {
    pub(crate) async fn attach_tab(&self, target_id: &TargetId) -> Result<(), Error> {
        let cell = self
            .shared
            .cells
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(target_id.clone())
            .or_default()
            .clone();

        let chromium = self.chromium.clone();
        let target_id = target_id.clone();

        // The cell is initialized in its own task: a caller that gives up after the request went
        // out only drops the handle, so the attach completes once and the next caller finds it.
        tokio::spawn(async move {
            cell.get_or_try_init(|| async {
                let result = chromium
                    .cdp()
                    .call(
                        None,
                        "Target.attachToTarget",
                        json!({ "targetId": target_id, "flatten": true }),
                    )
                    .await?;
                let session_id = result["sessionId"]
                    .as_str()
                    .ok_or_else(|| internal("Target.attachToTarget: no sessionId"))?;
                chromium
                    .init_attached(target_id.clone(), SessionId(session_id.to_owned()))
                    .await;

                Ok(())
            })
            .await
            .map(|_| ())
        })
        .await
        .map_err(internal)?
    }

    pub(crate) async fn navigate(
        &self,
        target_id: &TargetId,
        url: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<Navigated, Error> {
        self.attach_tab(target_id).await?;

        self.chromium
            .navigate(target_id, url, wait_until, timeout)
            .await
    }

    pub(crate) async fn activate_tab(&self, target_id: &TargetId) -> Result<(), Error> {
        self.attach_tab(target_id).await?;

        self.chromium.activate_tab(target_id).await
    }

    // Never a new window: the user's browser is not ours to rearrange.
    pub(crate) async fn open_tab(&self, url: &str) -> Result<TargetId, Error> {
        let _creating = self.shared.creating().await;
        let target_id = self.chromium.cdp().create_target(url, false).await?;
        self.shared
            .opened
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(target_id.clone());

        Ok(target_id)
    }

    async fn detach(&self) {
        // A browser that is already gone has nothing left to detach from.
        for session_id in self.chromium.session_ids() {
            let _ = self
                .chromium
                .cdp()
                .call(
                    None,
                    "Target.detachFromTarget",
                    json!({ "sessionId": session_id }),
                )
                .await;
        }
        self.stop.notify_one();
        // Pages and /cdp clients hold clones of the connection, so closing is explicit.
        self.chromium.cdp().close();
    }
}
