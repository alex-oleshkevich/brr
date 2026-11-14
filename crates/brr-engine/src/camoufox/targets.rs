use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use brr_cdp::SessionId;
use brr_core::{Size, TargetId};
use brr_juggler::{JugglerEvent, SessionId as JugglerSessionId};
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};
use tracing::warn;

use super::session::{init_session, translate};
use super::{Inner, PageSession, Tab};
use super::{downloads, intercept, network, screencast};
use crate::DriverEvent;
use crate::common::BLANK_PAGE;
use crate::page::{Page, PageState, Session};

const SCREENCAST_FRAME: &str = "Page.screencastFrame";
const TAB_INFO: &str = "({ url: location.href, title: document.title })";
const INFO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

type Ready = Arc<Mutex<Option<oneshot::Sender<TargetId>>>>;
// Load events of a Tab that is not attached yet, kept in order until TabAttached is sent: a
// Tab's first page can finish loading before its session is initialized, and a consumer that
// sees Loaded before Attached never reaches Ready.
type Held = Arc<AsyncMutex<HashMap<TargetId, Vec<DriverEvent>>>>;

struct Located {
    target_id: TargetId,
    session: PageSession,
    state: Arc<PageState>,
}

impl Inner {
    fn tab_of(&self, session_id: &JugglerSessionId) -> Option<Located> {
        self.tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(_, tab)| tab.session.juggler_session == *session_id)
            .map(|(target_id, tab)| Located {
                target_id: target_id.clone(),
                session: tab.session.clone(),
                state: tab.state.clone(),
            })
    }

    fn register(
        &self,
        session_id: JugglerSessionId,
        target_id: &TargetId,
        context_id: Option<String>,
    ) -> Located {
        let state = Arc::new(PageState::new(self.console_buffer, self.network_buffer));
        state.set_defaults(
            self.content_defaults
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        );
        let session = PageSession {
            juggler: self.juggler.clone(),
            session_id: SessionId(session_id.0.clone()),
            juggler_session: session_id,
            target_id: target_id.clone(),
            context_id,
            dialog_id: Arc::default(),
        };
        state.add_session(&session.session_id);
        self.tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                target_id.clone(),
                Tab {
                    session: session.clone(),
                    state: state.clone(),
                },
            );
        self.registered.notify_waiters();

        Located {
            target_id: target_id.clone(),
            session,
            state,
        }
    }
}

// Never awaits a Juggler call: the dispatcher feeds this loop, so a call awaited here could
// wait for a reply the loop itself is holding up.
pub(super) async fn run(
    inner: Arc<Inner>,
    mut events: mpsc::Receiver<JugglerEvent>,
    out: mpsc::Sender<DriverEvent>,
    ready: oneshot::Sender<TargetId>,
) {
    let ready: Ready = Arc::new(Mutex::new(Some(ready)));
    let held = Held::default();

    while let Some(event) = events.recv().await {
        let translated = match event {
            JugglerEvent::AttachedToTarget {
                session_id,
                target_id,
                browser_context_id,
                ..
            } => {
                let tab = inner.register(session_id, &target_id, browser_context_id);
                held.lock().await.insert(target_id.clone(), Vec::new());
                tokio::spawn(attach(
                    tab,
                    inner.profile_viewport.clone(),
                    out.clone(),
                    ready.clone(),
                    held.clone(),
                ));

                Some(DriverEvent::TabCreated {
                    target_id,
                    url: BLANK_PAGE.to_owned(),
                })
            }
            JugglerEvent::DetachedFromTarget { target_id, .. } => {
                held.lock().await.remove(&target_id);
                let tab = inner
                    .tabs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&target_id);
                // A screencast ends with its Tab.
                if let Some(tab) = tab {
                    tab.state.release_screencast();
                }

                Some(DriverEvent::TabDestroyed { target_id })
            }
            JugglerEvent::DownloadCreated {
                uuid,
                page_target_id,
                url,
                suggested_file_name,
            } => {
                for event in inner
                    .download_created(uuid, page_target_id, url, &suggested_file_name, &out)
                    .into_iter()
                    .flatten()
                {
                    let _ = out.send(event).await;
                }

                None
            }
            JugglerEvent::DownloadFinished {
                uuid,
                canceled,
                error,
            } => Some(inner.download_finished(uuid, canceled, error.as_deref())),
            JugglerEvent::Other {
                method,
                params,
                session_id: Some(session_id),
            } if method == SCREENCAST_FRAME => {
                if let Some(tab) = inner.tab_of(&session_id) {
                    screencast::frame(tab.session, tab.state, params);
                }

                None
            }
            JugglerEvent::Other {
                method,
                params,
                session_id: Some(session_id),
            } => inner
                .tab_of(&session_id)
                .and_then(|tab| page_event(&inner, &tab, &method, &params, &out)),
            _ => None,
        };

        let Some(event) = translated else {
            continue;
        };
        if let DriverEvent::TabLoading { target_id } | DriverEvent::TabLoaded { target_id } = &event
            && let Some(waiting) = held.lock().await.get_mut(target_id)
        {
            waiting.push(event);

            continue;
        }
        let _ = out.send(event).await;
    }
}

// Starts the Tab's init script, then reports it attached; the first one also resolves launch.
async fn attach(
    tab: Located,
    viewport: Option<Size>,
    out: mpsc::Sender<DriverEvent>,
    ready: Ready,
    held: Held,
) {
    if let Err(error) = init_session(&tab.session, tab.state.dismiss_cookies()).await {
        warn!(%error, "page session not initialized");
    }
    if tab.state.intercepting()
        && let Err(error) = tab.session.set_intercepting(true).await
    {
        warn!(%error, "page session not intercepted");
    }
    if let Some(size) = viewport
        && let Err(error) = tab.session.set_viewport(&size, None, false).await
    {
        warn!(%error, "profile viewport not applied");
    }
    {
        let mut held = held.lock().await;
        let _ = out
            .send(DriverEvent::TabAttached {
                target_id: tab.target_id.clone(),
            })
            .await;
        for event in held.remove(&tab.target_id).unwrap_or_default() {
            let _ = out.send(event).await;
        }
    }

    // Sent whatever failed above, so a launch reports the real problem, not a start timeout.
    if let Some(ready) = ready.lock().unwrap_or_else(PoisonError::into_inner).take() {
        let _ = ready.send(tab.target_id);
    }
}

fn page_event(
    inner: &Inner,
    tab: &Located,
    method: &str,
    params: &Value,
    out: &mpsc::Sender<DriverEvent>,
) -> Option<DriverEvent> {
    let session_id = &tab.session.session_id;
    let state = &tab.state;
    let target_id = tab.target_id.clone();
    let frame_id = params["frameId"].as_str().unwrap_or_default();
    let mut reported = None;

    match method {
        "Page.frameAttached"
            if params["parentFrameId"].is_null() && state.main_frame().is_empty() =>
        {
            state.seed(session_id, &json!({ "frame": { "id": frame_id } }));
        }
        "Page.dialogOpened" => {
            *tab.session
                .dialog_id
                .lock()
                .unwrap_or_else(PoisonError::into_inner) =
                params["dialogId"].as_str().map(str::to_owned);
        }
        "Page.navigationStarted" if state.is_main(frame_id) => {
            return Some(DriverEvent::TabLoading { target_id });
        }
        "Page.eventFired" if params["name"] == "load" && state.is_main(frame_id) => {
            tokio::spawn(report_info(tab.session.clone(), state.clone(), out.clone()));

            return Some(DriverEvent::TabLoaded { target_id });
        }
        "Page.eventFired" if params["name"] == "DOMContentLoaded" && state.is_main(frame_id) => {
            Page::new(Session::Camoufox(tab.session.clone()), state.clone()).inject_cosmetic();
        }
        "Page.sameDocumentNavigation" if state.is_main(frame_id) => {
            tokio::spawn(report_info(tab.session.clone(), state.clone(), out.clone()));
        }
        // The URL is known at commit; the title only once the document has loaded.
        "Page.navigationCommitted" if state.is_main(frame_id) => {
            reported = Some(DriverEvent::TabInfo {
                target_id,
                url: params["url"].as_str().unwrap_or_default().to_owned(),
                title: String::new(),
            });
        }
        "Page.crashed" => return Some(DriverEvent::TabCrashed { target_id }),
        "Network.requestWillBeSent" if params["isIntercepted"] == true => {
            intercept::paused(tab.session.clone(), state.clone(), params);
        }
        // A navigation or an attachment may turn into a download; Juggler names the size only
        // here, so it is kept until the download is announced.
        "Network.responseReceived" => {
            if let Some(length) = downloads::content_length(&params["headers"])
                && let Some((url, resource_type)) = params["requestId"]
                    .as_str()
                    .and_then(|request_id| state.request(request_id))
                && (resource_type == "Document" || downloads::is_attachment(&params["headers"]))
            {
                inner.remember_length(url, length);
            }
        }
        _ => {}
    }

    if let Some((method, params)) = network::translate(method, params) {
        state.apply(session_id, method, &params);
    }

    if let Some((method, params)) = translate(method, params)
        && PageState::tracks(method)
    {
        state.apply(session_id, method, &params);
    }

    reported
}

// Juggler reports no title; the page is asked once its document has loaded.
async fn report_info(session: PageSession, state: Arc<PageState>, out: mpsc::Sender<DriverEvent>) {
    let target_id = session.target_id.clone();
    let page = Page::new(Session::Camoufox(session), state);
    let info = match page.evaluate_internal(TAB_INFO, INFO_TIMEOUT).await {
        Ok(info) => info,
        Err(error) => {
            warn!(%error, "reading a loaded tab failed");
            return;
        }
    };

    let _ = out
        .send(DriverEvent::TabInfo {
            target_id,
            url: info["url"].as_str().unwrap_or_default().to_owned(),
            title: info["title"].as_str().unwrap_or_default().to_owned(),
        })
        .await;
}
