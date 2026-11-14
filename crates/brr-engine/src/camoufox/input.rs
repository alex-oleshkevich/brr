use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use brr_core::{Error, WaitUntil};
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::{PageSession, gone, internal};
use crate::chromium::millis;
use crate::common::CONNECTION_CLOSED;

// CDP and Firefox number the modifier bits differently: Meta and Shift swap places.
const CDP_META: u64 = 4;
const CDP_SHIFT: u64 = 8;
const FIREFOX_SHIFT: u64 = 4;
const FIREFOX_META: u64 = 8;
const CARRIAGE_RETURN: &str = "\r";

pub(super) fn modifiers(cdp: &Value) -> u64 {
    let mask = cdp.as_u64().unwrap_or_default();
    let shared = mask & !(CDP_META | CDP_SHIFT);

    shared
        | if mask & CDP_SHIFT != 0 {
            FIREFOX_SHIFT
        } else {
            0
        }
        | if mask & CDP_META != 0 {
            FIREFOX_META
        } else {
            0
        }
}

fn button(name: &Value) -> u64 {
    match name.as_str() {
        Some("middle") => 1,
        Some("right") => 2,
        _ => 0,
    }
}

#[derive(Default)]
struct Aborts {
    expected: Option<String>,
    // Aborts seen before the navigation's id was known: (id, error text).
    seen: Vec<(String, String)>,
}

// A load event counts only for a document this navigation committed: the page being left may
// still be loading, and its events arrive after the wait is registered.
#[derive(Default)]
struct Commits {
    // The navigation that last committed in the main frame since the wait began.
    current: Option<String>,
    expected: Option<String>,
    // History and reload ids are not reported by the call: any commit counts.
    any: bool,
    // Navigations whose load was seen before the expected id was known.
    loaded: Vec<String>,
}

struct Waiters {
    loaded: oneshot::Receiver<Value>,
    same_document: oneshot::Receiver<Value>,
    aborted: oneshot::Receiver<Value>,
    // Held so the matcher that records commits keeps running; it never resolves.
    _committed: oneshot::Receiver<Value>,
    aborts: Arc<Mutex<Aborts>>,
    commits: Arc<Mutex<Commits>>,
}

fn event_name(wait_until: WaitUntil) -> &'static str {
    match wait_until {
        WaitUntil::Domcontentloaded => "DOMContentLoaded",
        // Network idle is awaited by the caller, after the load event.
        WaitUntil::Load | WaitUntil::Networkidle => "load",
    }
}

impl PageSession {
    // `params` are the CDP-shaped mouse event the shared actions build.
    pub(crate) async fn dispatch_mouse(&self, params: &Value) -> Result<(), Error> {
        let mask = modifiers(&params["modifiers"]);

        if params["type"] == "mouseWheel" {
            self.call(
                "Page.dispatchWheelEvent",
                json!({
                    "x": params["x"],
                    "y": params["y"],
                    "deltaX": params["deltaX"],
                    "deltaY": params["deltaY"],
                    "deltaZ": 0,
                    "modifiers": mask,
                }),
            )
            .await?;

            return Ok(());
        }

        let kind = match params["type"].as_str() {
            Some("mousePressed") => "mousedown",
            Some("mouseReleased") => "mouseup",
            _ => "mousemove",
        };
        self.call(
            "Page.dispatchMouseEvent",
            json!({
                "type": kind,
                "button": button(&params["button"]),
                "x": params["x"],
                "y": params["y"],
                "modifiers": mask,
                "clickCount": params["clickCount"],
                "buttons": params["buttons"],
            }),
        )
        .await?;

        Ok(())
    }

    pub(crate) async fn dispatch_key(&self, params: &Value) -> Result<(), Error> {
        let up = params["type"] == "keyUp";
        let mut event = json!({
            "type": if up { "keyup" } else { "keydown" },
            "key": params["key"],
            "keyCode": params["windowsVirtualKeyCode"],
            "code": params["code"],
            "location": params["location"].as_u64().unwrap_or_default(),
            "repeat": params["autoRepeat"].as_bool().unwrap_or(false),
        });
        // Firefox works out Enter by itself.
        if let Some(text) = params["text"].as_str()
            && !up
            && !text.is_empty()
            && text != CARRIAGE_RETURN
        {
            event["text"] = json!(text);
        }
        self.call("Page.dispatchKeyEvent", event).await?;

        Ok(())
    }

    pub(crate) async fn insert_text(&self, text: &str) -> Result<(), Error> {
        self.call("Page.insertText", json!({ "text": text }))
            .await?;

        Ok(())
    }

    pub(crate) async fn handle_dialog(
        &self,
        accept: bool,
        prompt_text: Option<&str>,
    ) -> Result<(), Error> {
        let dialog_id = self
            .dialog_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| gone("open dialog"))?;
        let mut params = json!({ "dialogId": dialog_id, "accept": accept });
        if let Some(prompt_text) = prompt_text {
            params["promptText"] = json!(prompt_text);
        }
        self.call("Page.handleDialog", params).await?;

        Ok(())
    }

    // The main frame's next document load, or its same-document navigation, ends a wait that was
    // registered before the call that causes it.
    fn navigation_waiters(&self, main_frame: &str, wait_until: WaitUntil, any: bool) -> Waiters {
        let wanted = event_name(wait_until);
        let (loaded_frame, same_frame, aborted_frame, committed_frame) = (
            main_frame.to_owned(),
            main_frame.to_owned(),
            main_frame.to_owned(),
            main_frame.to_owned(),
        );
        let commits = Arc::new(Mutex::new(Commits {
            any,
            ..Commits::default()
        }));
        let (on_load, on_commit) = (commits.clone(), commits.clone());
        let aborts = Arc::new(Mutex::new(Aborts::default()));
        let recorded = aborts.clone();

        Waiters {
            loaded: self.juggler.wait_for(
                Some(&self.juggler_session),
                "Page.eventFired",
                Box::new(move |params| {
                    if params["frameId"] != loaded_frame || params["name"] != wanted {
                        return false;
                    }
                    let mut commits = on_load.lock().unwrap_or_else(PoisonError::into_inner);
                    let Some(current) = commits.current.clone() else {
                        return false;
                    };

                    match &commits.expected {
                        _ if commits.any => true,
                        Some(expected) => *expected == current,
                        None => {
                            commits.loaded.push(current);
                            false
                        }
                    }
                }),
            ),
            _committed: self.juggler.wait_for(
                Some(&self.juggler_session),
                "Page.navigationCommitted",
                Box::new(move |params| {
                    if params["frameId"] == committed_frame {
                        on_commit
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .current = params["navigationId"].as_str().map(str::to_owned);
                    }

                    false
                }),
            ),
            same_document: self.juggler.wait_for(
                Some(&self.juggler_session),
                "Page.sameDocumentNavigation",
                Box::new(move |params| params["frameId"] == same_frame),
            ),
            // Navigating aborts the navigation in flight, so only our own id counts; an abort
            // that arrives before navigate answers is kept until the id is known.
            aborted: self.juggler.wait_for(
                Some(&self.juggler_session),
                "Page.navigationAborted",
                Box::new(move |params| {
                    if params["frameId"] != aborted_frame {
                        return false;
                    }
                    let id = params["navigationId"].as_str().unwrap_or_default();
                    let mut aborts = recorded.lock().unwrap_or_else(PoisonError::into_inner);

                    match &aborts.expected {
                        Some(expected) => expected == id,
                        None => {
                            let error = params["errorText"].as_str().unwrap_or_default();
                            aborts.seen.push((id.to_owned(), error.to_owned()));
                            false
                        }
                    }
                }),
            ),
            aborts,
            commits,
        }
    }

    async fn await_navigation(waiters: Waiters, timeout: Duration) -> Result<(), Error> {
        let Waiters {
            loaded,
            same_document,
            aborted,
            ..
        } = waiters;
        let closed = |_| internal(CONNECTION_CLOSED);

        tokio::time::timeout(timeout, async {
            tokio::select! {
                result = loaded => result.map(|_| ()).map_err(closed),
                result = same_document => result.map(|_| ()).map_err(closed),
                result = aborted => {
                    let aborted = result.map_err(closed)?;
                    Err(Error::NavigationFailed {
                        net_error: aborted["errorText"].as_str().unwrap_or_default().to_owned(),
                    })
                }
            }
        })
        .await
        .map_err(|_| Error::Timeout {
            after_ms: millis(timeout),
        })?
    }

    pub(crate) async fn navigate(
        &self,
        main_frame: &str,
        url: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<(), Error> {
        let waiters = self.navigation_waiters(main_frame, wait_until, false);

        let result = tokio::time::timeout(
            timeout,
            self.juggler
                .navigate(&self.juggler_session, main_frame, url),
        )
        .await
        .map_err(|_| Error::Timeout {
            after_ms: millis(timeout),
        })??;
        // Same-document navigation starts no load.
        let Some(navigation_id) = result.navigation_id else {
            return Ok(());
        };
        {
            let mut aborts = waiters
                .aborts
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some((_, error)) = aborts.seen.iter().find(|(id, _)| *id == navigation_id) {
                return Err(Error::NavigationFailed {
                    net_error: error.clone(),
                });
            }
            aborts.expected = Some(navigation_id.clone());
        }
        {
            let mut commits = waiters
                .commits
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if commits.loaded.contains(&navigation_id) {
                return Ok(());
            }
            commits.expected = Some(navigation_id);
        }

        Self::await_navigation(waiters, timeout).await
    }

    // Ok(false) when there is no entry that way.
    pub(crate) async fn go_history(
        &self,
        delta: i64,
        main_frame: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<bool, Error> {
        let waiters = self.navigation_waiters(main_frame, wait_until, true);
        let method = if delta < 0 {
            "Page.goBack"
        } else {
            "Page.goForward"
        };
        let result = self.call(method, json!({ "frameId": main_frame })).await?;
        if result["success"] != true {
            return Ok(false);
        }

        Self::await_navigation(waiters, timeout).await?;

        Ok(true)
    }

    pub(crate) async fn reload(
        &self,
        main_frame: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<(), Error> {
        let waiters = self.navigation_waiters(main_frame, wait_until, true);
        self.call("Page.reload", json!({})).await?;

        Self::await_navigation(waiters, timeout).await
    }
}
