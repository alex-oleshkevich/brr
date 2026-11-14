use std::sync::{Arc, Mutex, PoisonError};

use brr_cdp::{Cdp, SessionId, TargetInfo};
use brr_core::{Error, TargetId};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use super::fetch;
use super::{Inner, Tab};
use crate::DriverEvent;
use crate::page::content::register_consent;
use crate::page::{Page, PageSession, PageState, Session, create_world};

pub(super) const PAGE: &str = "page";
pub(super) const IFRAME: &str = "iframe";
const WORKER: &str = "worker";
// A /cdp client's own attachToBrowserTarget session; it is never a page.
const BROWSER: &str = "browser";
const READY_STATE_COMPLETE: &str = "complete";

pub(super) type Ready = Arc<Mutex<Option<oneshot::Sender<TargetId>>>>;

// Enables what the engine reads on a page or iframe session and seeds its frames, returning their
// ids. The caller resumes the target afterwards: a child session is paused until then, so its
// first document is never missed.
pub(super) async fn init_session(
    cdp: &Cdp,
    session_id: &SessionId,
    state: &PageState,
) -> Result<Vec<String>, Error> {
    cdp.enable_page(session_id).await?;
    cdp.call(Some(session_id), "Runtime.enable", json!({}))
        .await?;
    cdp.call(Some(session_id), "Network.enable", json!({}))
        .await?;
    cdp.call(Some(session_id), "Log.enable", json!({})).await?;
    fetch::enable_if_intercepting(cdp, session_id, state).await?;
    if state.dismiss_cookies() {
        register_consent(cdp, session_id, state).await?;
    }

    let mut tree = cdp
        .call(Some(session_id), "Page.getFrameTree", json!({}))
        .await?;
    let frame_ids = state.seed(session_id, &tree["frameTree"].take());

    cdp.set_auto_attach(Some(session_id)).await?;

    Ok(frame_ids)
}

// Chromium never answers Page.createIsolatedWorld in an out-of-process iframe that is still
// paused, so those get their worlds after the resume.
async fn create_worlds(
    cdp: &Cdp,
    session_id: &SessionId,
    frame_ids: Vec<String>,
    state: &PageState,
) {
    for frame_id in frame_ids {
        if let Err(error) = create_world(cdp, session_id, &frame_id, state).await {
            warn!(%error, frame_id, "utility world not created");
        }
    }
}

// Page and iframe sessions get an engine state; every other target kind (workers) is only resumed.
// Runs in its own task: the event loop must never wait on a call.
pub(super) async fn attach_other(
    inner: Arc<Inner>,
    session_id: SessionId,
    info: TargetInfo,
) -> Result<(), Error> {
    if info.kind == BROWSER {
        return Ok(());
    }

    if info.kind == WORKER {
        let state = inner
            .tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .find(|tab| {
                info.parent_frame_id
                    .as_deref()
                    .is_some_and(|frame_id| tab.state.owns_frame(frame_id))
            })
            .map(|tab| tab.state.clone());
        if let Some(state) = state {
            state.add_session(&session_id);
            inner
                .routes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(session_id.clone(), state.clone());

            if let Err(error) = fetch::enable_if_intercepting(&inner.cdp, &session_id, &state).await
            {
                warn!(%error, "worker session not intercepted");
            }
        }
    }

    if info.kind == IFRAME {
        let state = inner
            .tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .find(|tab| {
                tab.state.adopt(
                    &info.target_id.0,
                    &session_id,
                    info.parent_frame_id.as_deref(),
                )
            })
            .map(|tab| tab.state.clone());
        let Some(state) = state else {
            return inner.cdp.run_if_waiting(&session_id).await;
        };
        inner
            .routes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(session_id.clone(), state.clone());
        state.add_session(&session_id);

        let frame_ids = init_session(&inner.cdp, &session_id, &state)
            .await
            .unwrap_or_else(|error| {
                warn!(%error, "iframe session not initialized");
                Vec::new()
            });
        inner.cdp.run_if_waiting(&session_id).await?;
        create_worlds(&inner.cdp, &session_id, frame_ids, &state).await;

        return Ok(());
    }

    inner.cdp.run_if_waiting(&session_id).await
}

// The tab is registered before this runs, so events during init already route to its state.
pub(super) async fn attach_page(
    inner: Arc<Inner>,
    events: mpsc::Sender<DriverEvent>,
    ready: Ready,
    resume: bool,
    session_id: SessionId,
    target_id: TargetId,
    state: Arc<PageState>,
) {
    match init_session(&inner.cdp, &session_id, &state).await {
        Ok(frame_ids) => create_worlds(&inner.cdp, &session_id, frame_ids, &state).await,
        Err(error) => warn!(%error, "page session not initialized"),
    }
    if inner.profile_view.viewport.is_some() || inner.profile_view.geolocation.is_some() {
        let page = Page::new(
            Session::Chromium(PageSession {
                cdp: inner.cdp.clone(),
                session_id: session_id.clone(),
                target_id: target_id.clone(),
            }),
            state.clone(),
        );
        if let Err(error) = page.emulate(&inner.profile_view).await {
            warn!(%error, "profile viewport or geolocation not applied");
        }
    }
    let _ = events
        .send(DriverEvent::TabAttached {
            target_id: target_id.clone(),
        })
        .await;
    // A Live page was never paused.
    if resume && let Err(error) = inner.cdp.run_if_waiting(&session_id).await {
        warn!(%error, "resuming a page failed");
    }

    // The load event can fire while the session initializes, before the owner has seen the
    // attach and so cannot use it, and a target that existed before auto-attach fired it before
    // we listened. Either way the page is complete by now.
    match inner.cdp.ready_state(&session_id).await {
        Ok(state) if state == READY_STATE_COMPLETE => {
            let _ = events
                .send(DriverEvent::TabLoaded {
                    target_id: target_id.clone(),
                })
                .await;
        }
        Ok(_) => {}
        Err(error) => warn!(%error, "reading the ready state failed"),
    }

    // Sent whatever failed above, so a launch reports the real problem, not a start timeout.
    if let Some(ready) = ready.lock().unwrap_or_else(PoisonError::into_inner).take() {
        let _ = ready.send(target_id);
    }
}

pub(super) fn register_tab(
    inner: &Inner,
    session_id: &SessionId,
    target_id: &TargetId,
) -> Arc<PageState> {
    let state = Arc::new(PageState::new(inner.console_buffer, inner.network_buffer));
    state.set_defaults(
        inner
            .content_defaults
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
    );
    state.add_session(session_id);
    inner
        .tabs
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            target_id.clone(),
            Tab {
                session_id: session_id.clone(),
                state: state.clone(),
            },
        );
    inner
        .routes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(session_id.clone(), state.clone());

    state
}
