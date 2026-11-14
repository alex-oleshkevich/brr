use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use brr_cdp::{CdpEvent, SessionId, TargetInfo};
use brr_core::{Error, TargetId};
use serde_json::json;
use tokio::sync::{Notify, mpsc, oneshot};
use tracing::warn;

use super::Inner;
use super::downloads;
use super::fetch;
use super::screencast;
use super::session::{PAGE, Ready, attach_other, attach_page, register_tab};
use crate::DriverEvent;
use crate::common::EVENTS_QUEUE;
use crate::live::{LiveTarget, Shared};
use crate::page::{Page, PageSession, PageState, Session};

const FETCH_PAUSED: &str = "Fetch.requestPaused";
const SCREENCAST_FRAME: &str = "Page.screencastFrame";
const DOM_CONTENT_LOADED: &str = "DOMContentLoaded";

// A launched browser attaches to every page it creates and reports the first one; a Live browser
// is the user's, so its pages are only announced and attached when a Tab is first used.
pub(super) enum Mode {
    Launch(oneshot::Sender<TargetId>),
    Live {
        stop: Arc<Notify>,
        // The pages discovered before the loop started; announced like any later one.
        initial: Vec<LiveTarget>,
        shared: Arc<Shared>,
    },
}

pub(super) async fn run(
    inner: Arc<Inner>,
    mut cdp_events: mpsc::Receiver<CdpEvent>,
    events: mpsc::Sender<DriverEvent>,
    mode: Mode,
) {
    // This loop feeds the dispatcher's reader, so it never awaits a CDP call or a lock a CDP call
    // holds: that work runs in its own task, since the reply it waits for is read behind this loop.
    let (decided, mut decided_rx) = mpsc::channel(EVENTS_QUEUE);
    let mut discovery = Discovery {
        announced: HashSet::new(),
        pending: HashMap::new(),
        decided,
    };
    let mut pages = HashSet::new();
    let (ready, live) = match mode {
        Mode::Launch(ready) => (Some(ready), None),
        Mode::Live {
            stop,
            initial,
            shared,
        } => {
            for target in initial {
                announce(&events, &mut discovery.announced, target).await;
            }

            (None, Some((stop, shared)))
        }
    };
    let ready: Ready = Arc::new(Mutex::new(ready));

    loop {
        let received = match &live {
            Some((stop, _)) => tokio::select! {
                event = cdp_events.recv() => event,
                Some((target_id, shown)) = decided_rx.recv() => {
                    discovery.resolve(&events, target_id, shown).await;
                    continue;
                }
                () = stop.notified() => return,
            },
            None => cdp_events.recv().await,
        };
        let Some(event) = received else { break };

        if let Some((_, shared)) = &live
            && live_event(&inner, &events, &mut discovery, shared, &event).await
        {
            continue;
        }

        let translated = match event {
            CdpEvent::TargetCreated(info) if info.kind == PAGE => {
                pages.insert(info.target_id.clone());
                Some(DriverEvent::TabCreated {
                    target_id: info.target_id,
                    url: info.url,
                })
            }
            CdpEvent::TargetInfoChanged(info) if info.kind == PAGE => Some(DriverEvent::TabInfo {
                target_id: info.target_id,
                url: info.url,
                title: info.title,
            }),
            // Frames, workers and service workers are targets too, but not Tabs.
            CdpEvent::TargetCrashed { target_id } => pages
                .contains(&target_id)
                .then_some(DriverEvent::TabCrashed { target_id }),
            CdpEvent::TargetDestroyed { target_id } => {
                inner.forget(&target_id);
                pages
                    .remove(&target_id)
                    .then_some(DriverEvent::TabDestroyed { target_id })
            }
            CdpEvent::DetachedFromTarget { session_id } => {
                if let Some(state) = inner
                    .routes
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&session_id)
                {
                    state.session_detached(&session_id);
                }
                None
            }
            CdpEvent::Other {
                method,
                params,
                session_id: Some(session_id),
            } if PageState::tracks(&method) => {
                let state = inner
                    .routes
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&session_id)
                    .cloned();
                if let Some(state) = state {
                    state.apply(&session_id, &method, &params);
                }
                None
            }
            CdpEvent::Other {
                method,
                params,
                session_id: Some(session_id),
            } if method == FETCH_PAUSED => {
                let state = inner
                    .routes
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&session_id)
                    .cloned();
                if let Some(state) = state {
                    fetch::paused(inner.cdp.clone(), session_id, state, params);
                }
                None
            }
            CdpEvent::Other {
                method,
                params,
                session_id: Some(session_id),
            } if method == SCREENCAST_FRAME => {
                let state = inner
                    .routes
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&session_id)
                    .cloned();
                if let Some(state) = state {
                    screencast::frame(inner.cdp.clone(), session_id, state, params);
                }
                None
            }
            CdpEvent::Other {
                method,
                params,
                session_id: None,
            } if downloads::is_download_event(&method) => {
                downloads::translate(&inner, &method, &params)
            }
            CdpEvent::FrameStartedLoading {
                session_id,
                frame_id,
            } => inner
                .target_of(&session_id)
                .filter(|target_id| target_id.0 == frame_id)
                .map(|target_id| DriverEvent::TabLoading { target_id }),
            CdpEvent::LifecycleEvent {
                session_id,
                frame_id,
                name,
                ..
            } if name == DOM_CONTENT_LOADED => {
                if let Some(target_id) = inner.target_of(&session_id)
                    && let Some(tab) = inner
                        .tabs
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&target_id)
                        .filter(|tab| tab.state.is_main(&frame_id))
                {
                    let page = Page::new(
                        Session::Chromium(PageSession {
                            cdp: inner.cdp.clone(),
                            session_id,
                            target_id,
                        }),
                        tab.state.clone(),
                    );
                    page.inject_cosmetic();
                }
                None
            }
            CdpEvent::LoadEventFired { session_id } => {
                if let Some(target_id) = inner.target_of(&session_id) {
                    let _ = events
                        .send(DriverEvent::TabLoaded {
                            target_id: target_id.clone(),
                        })
                        .await;
                    tokio::spawn(read_title(inner.clone(), events.clone(), target_id));
                }
                None
            }
            CdpEvent::AttachedToTarget {
                session_id,
                target_info,
                ..
            } => {
                if target_info.kind == PAGE {
                    let target_id = target_info.target_id;
                    pages.insert(target_id.clone());
                    let state = register_tab(&inner, &session_id, &target_id);
                    tokio::spawn(attach_page(
                        inner.clone(),
                        events.clone(),
                        ready.clone(),
                        true,
                        session_id,
                        target_id,
                        state,
                    ));
                } else {
                    let attach = attach_other(inner.clone(), session_id, target_info);
                    tokio::spawn(async move {
                        if let Err(error) = attach.await {
                            warn!(%error, "attaching to a target failed");
                        }
                    });
                }
                None
            }
            _ => None,
        };

        if let Some(event) = translated {
            let _ = events.send(event).await;
        }
    }

    // A launched browser's exit comes from its process; a Live one is only known to be gone
    // when the connection ends.
    if live.is_some() {
        let _ = events
            .send(DriverEvent::Exited {
                code: None,
                signal: None,
            })
            .await;
    }
}

const HIDDEN_URL_PREFIXES: [&str; 6] = [
    "chrome://",
    "chrome-untrusted://",
    "devtools://",
    "chrome-extension://",
    "edge://",
    "brave://",
];

pub(crate) fn is_user_page(info: &TargetInfo) -> bool {
    info.kind == PAGE && is_user_page_url(&info.url)
}

fn is_user_page_url(url: &str) -> bool {
    !HIDDEN_URL_PREFIXES
        .iter()
        .any(|prefix| url.starts_with(prefix))
}

// Live pages have no lifecycle events until attached, so a discovered page counts as attached
// and loaded.
async fn announce(
    events: &mpsc::Sender<DriverEvent>,
    announced: &mut HashSet<TargetId>,
    target: LiveTarget,
) {
    announced.insert(target.target_id.clone());
    for event in [
        DriverEvent::TabCreated {
            target_id: target.target_id.clone(),
            url: target.url.clone(),
        },
        DriverEvent::TabAttached {
            target_id: target.target_id.clone(),
        },
        DriverEvent::TabLoaded {
            target_id: target.target_id.clone(),
        },
        DriverEvent::TabInfo {
            target_id: target.target_id,
            url: target.url,
            title: target.title,
        },
    ] {
        let _ = events.send(event).await;
    }
}

// What the Live loop knows of the pages it has shown, and of those waiting on `creating`.
struct Discovery {
    announced: HashSet<TargetId>,
    // Pages whose announcement waits for a page brr is creating; the latest info is kept.
    pending: HashMap<TargetId, LiveTarget>,
    decided: mpsc::Sender<(TargetId, bool)>,
}

impl Discovery {
    async fn resolve(
        &mut self,
        events: &mpsc::Sender<DriverEvent>,
        target_id: TargetId,
        shown: bool,
    ) {
        // A page destroyed meanwhile is no longer pending.
        let Some(target) = self.pending.remove(&target_id) else {
            return;
        };
        if shown {
            announce(events, &mut self.announced, target).await;
        }
    }

    // Waits for a page brr is creating to be recorded as brr's. Whatever URL a page brr opened
    // has, it was asked for, so it is announced.
    fn decide(&mut self, shared: &Arc<Shared>, target: LiveTarget) {
        let target_id = target.target_id.clone();
        let user_page = is_user_page_url(&target.url);
        if self.pending.insert(target_id.clone(), target).is_some() {
            return;
        }

        let shared = shared.clone();
        let decided = self.decided.clone();
        tokio::spawn(async move {
            let shown = {
                let _creating = shared.creating().await;
                user_page || shared.is_opened(&target_id)
            };
            let _ = decided.send((target_id, shown)).await;
        });
    }
}

// Returns whether the event was consumed. A Tab already shown keeps reporting its info even when
// it moves to a hidden URL; only a page not yet shown is held back by the URL filter.
async fn live_event(
    inner: &Inner,
    events: &mpsc::Sender<DriverEvent>,
    discovery: &mut Discovery,
    shared: &Arc<Shared>,
    event: &CdpEvent,
) -> bool {
    match event {
        CdpEvent::TargetCreated(info) | CdpEvent::TargetInfoChanged(info) if info.kind == PAGE => {
            let target = LiveTarget {
                target_id: info.target_id.clone(),
                url: info.url.clone(),
                title: info.title.clone(),
            };
            if discovery.announced.contains(&info.target_id) {
                if matches!(event, CdpEvent::TargetInfoChanged(_)) {
                    let _ = events
                        .send(DriverEvent::TabInfo {
                            target_id: target.target_id,
                            url: target.url,
                            title: target.title,
                        })
                        .await;
                }
            } else {
                discovery.decide(shared, target);
            }

            true
        }
        CdpEvent::TargetDestroyed { target_id } => {
            inner.forget(target_id);
            shared.forget(target_id);
            discovery.pending.remove(target_id);
            if discovery.announced.remove(target_id) {
                let _ = events
                    .send(DriverEvent::TabDestroyed {
                        target_id: target_id.clone(),
                    })
                    .await;
            }

            true
        }
        CdpEvent::TargetCrashed { target_id } => {
            if discovery.announced.contains(target_id) {
                let _ = events
                    .send(DriverEvent::TabCrashed {
                        target_id: target_id.clone(),
                    })
                    .await;
            }

            true
        }
        // attach_tab registers the page itself, so the session exists before its first event.
        CdpEvent::AttachedToTarget { target_info, .. } => target_info.kind == PAGE,
        _ => false,
    }
}

// Chromium sends no Target.targetInfoChanged for a title set while the page loads, so the title
// is read once the load event fires.
async fn read_title(inner: Arc<Inner>, events: mpsc::Sender<DriverEvent>, target_id: TargetId) {
    match inner.target_info(&target_id).await {
        Ok(info) => {
            let _ = events
                .send(DriverEvent::TabInfo {
                    target_id,
                    url: info.url,
                    title: info.title,
                })
                .await;
        }
        Err(error) => warn!(%error, "reading a loaded tab failed"),
    }
}

impl Inner {
    async fn target_info(&self, target_id: &TargetId) -> Result<TargetInfo, Error> {
        let method = "Target.getTargetInfo";
        let result = self
            .cdp
            .call(None, method, json!({ "targetId": target_id }))
            .await?;

        serde_json::from_value(result["targetInfo"].clone()).map_err(|error| Error::Internal {
            message: format!("{method}: unexpected response: {error}"),
        })
    }

    fn target_of(&self, session_id: &SessionId) -> Option<TargetId> {
        self.tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(_, tab)| tab.session_id == *session_id)
            .map(|(target_id, _)| target_id.clone())
    }

    fn forget(&self, target_id: &TargetId) {
        let tab = self
            .tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(target_id);
        if let Some(tab) = tab {
            // A screencast ends with its Tab.
            tab.state.release_screencast();
            self.routes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|_, state| !Arc::ptr_eq(state, &tab.state));
        }
    }
}
