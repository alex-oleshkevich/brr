use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use brr_core::Error;
use brr_proto::{FORWARDED, Incoming};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::Cdp;

#[cfg(test)]
mod tests;

pub const DOWNSTREAM_QUEUE: usize = 1024;

const SESSION_NOT_FOUND: i64 = -32001;
const SESSION_NOT_FOUND_MESSAGE: &str = "Session with given id not found.";

#[derive(Debug, PartialEq, Eq)]
pub enum Outbound {
    Frame(String),
    BrowserClose,
}

struct Route {
    // None once the Client was dropped: the route stays as a tombstone so its frames are
    // discarded instead of falling through to brr's own decoder.
    tx: Option<mpsc::Sender<Outbound>>,
    // The Client's browser-level session; the key of its own route.
    browser: String,
}

#[derive(Clone, Default)]
pub(crate) struct Routes(Arc<Mutex<HashMap<String, Route>>>);

impl Routes {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Route>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // Sent through the dispatcher so every frame queued before it is routed first.
    pub(crate) fn remove_client(&self, browser: &str) {
        self.lock().retain(|_, route| route.browser != browser);
    }

    pub(crate) fn clear(&self) {
        self.lock().clear();
    }

    pub(crate) fn push(&self, browser: &str, outbound: Outbound) {
        let tx = self.lock().get(browser).and_then(|route| route.tx.clone());
        if let Some(tx) = tx {
            self.deliver(browser, &tx, outbound);
        }
    }

    // A full queue must never stall the dispatcher: the Client is dropped instead.
    fn deliver(&self, browser: &str, tx: &mpsc::Sender<Outbound>, outbound: Outbound) {
        match tx.try_send(outbound) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!(
                    queue = DOWNSTREAM_QUEUE,
                    "downstream client is too slow; dropping it"
                );
                self.tombstone(browser);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => self.tombstone(browser),
        }
    }

    pub(crate) fn tombstone(&self, browser: &str) {
        for route in self.lock().values_mut() {
            if route.browser == browser {
                route.tx = None;
            }
        }
    }

    fn owns(&self, browser: &str, session: &str) -> bool {
        self.lock()
            .get(session)
            .is_some_and(|route| route.browser == browser)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventFrame<'a> {
    method: &'a str,
    params: &'a Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
}

// The answer to a Client's command; the result is borrowed, it can be megabytes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponseFrame<'a> {
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
}

// Returns the frame when it belongs to brr itself.
pub(crate) fn route(routes: &Routes, frame: Incoming) -> Option<Incoming> {
    if frame.method == FORWARDED {
        forwarded(routes, &frame.params);
        return None;
    }
    let Some(session) = frame.session_id.as_deref() else {
        return Some(frame);
    };
    let child = |params: &Value| params["sessionId"].as_str().map(str::to_owned);
    let (browser, tx) = {
        let mut table = routes.lock();
        let Some(route) = table.get(session) else {
            return Some(frame);
        };
        let browser = route.browser.clone();
        let tx = route.tx.clone();

        // Registered before forwarding: the Client answers an attach on that child immediately.
        if frame.method == "Target.attachedToTarget"
            && let Some(new) = child(&frame.params)
        {
            table.insert(
                new,
                Route {
                    tx: tx.clone(),
                    browser: browser.clone(),
                },
            );
        }
        (browser, tx)
    };

    if let Some(tx) = tx {
        let forwarded = EventFrame {
            method: &frame.method,
            params: &frame.params,
            session_id: (session != browser).then_some(session),
        };
        match serde_json::to_string(&forwarded) {
            Ok(text) => routes.deliver(&browser, &tx, Outbound::Frame(text)),
            Err(error) => debug!(%error, "unserializable downstream event"),
        }
    }

    if frame.method == "Target.detachedFromTarget"
        && let Some(gone) = child(&frame.params)
    {
        routes.lock().remove(&gone);
    }

    None
}

// The browser's answer to a Client's command, in stream order with the events around it.
fn forwarded(routes: &Routes, params: &Value) {
    let context = &params["context"];
    let (Some(browser), Some(id)) = (context["browser"].as_str(), context["id"].as_u64()) else {
        return;
    };
    let error = params.get("error");
    let frame = response_frame(&ResponseFrame {
        id,
        result: error.is_none().then_some(&params["result"]),
        error,
        session_id: context["session"].as_str(),
    });
    routes.push(browser, Outbound::Frame(frame));
}

struct Call {
    method: String,
    params: Value,
    target: String,
    context: Value,
}

pub struct Downstream {
    cdp: Cdp,
    // One task writes a Client's commands in the order it sent them; separate tasks would let a
    // later command reach the browser first.
    calls: mpsc::Sender<Call>,
    browser: String,
    removed: bool,
}

impl Cdp {
    pub async fn attach_downstream(&self) -> Result<(Downstream, mpsc::Receiver<Outbound>), Error> {
        let result = self
            .call(None, "Target.attachToBrowserTarget", json!({}))
            .await?;
        let browser = result["sessionId"]
            .as_str()
            .ok_or_else(|| Error::Internal {
                message: "Target.attachToBrowserTarget returned no sessionId".to_owned(),
            })?
            .to_owned();
        let (calls, mut pending) = mpsc::channel::<Call>(DOWNSTREAM_QUEUE);
        let mux = self.mux.clone();
        tokio::spawn(async move {
            while let Some(call) = pending.recv().await {
                let sent = mux
                    .forward(&call.method, call.params, Some(&call.target), call.context)
                    .await;
                if sent.is_err() {
                    break;
                }
            }
        });
        let (tx, rx) = mpsc::channel(DOWNSTREAM_QUEUE);
        self.routes.lock().insert(
            browser.clone(),
            Route {
                tx: Some(tx),
                browser: browser.clone(),
            },
        );

        Ok((
            Downstream {
                cdp: self.clone(),
                calls,
                browser,
                removed: false,
            },
            rx,
        ))
    }
}

impl Downstream {
    pub fn send(&self, text: &str) {
        let mut message: Value = match serde_json::from_str(text) {
            Ok(message) => message,
            Err(error) => {
                debug!(%error, "unparsable downstream frame");
                return;
            }
        };
        let Some(id) = message["id"].as_u64() else {
            debug!("downstream frame without id");
            return;
        };
        let method = message["method"].as_str().unwrap_or_default().to_owned();
        let params = match message["params"].take() {
            Value::Null => json!({}),
            params => params,
        };
        let session = message["sessionId"].as_str().map(str::to_owned);
        let routes = &self.cdp.routes;

        if matches!(
            method.as_str(),
            "Browser.close" | "Browser.setDownloadBehavior" | "Page.setDownloadBehavior"
        ) {
            routes.push(
                &self.browser,
                Outbound::Frame(result_frame(id, &json!({}), session.as_deref())),
            );
            if method == "Browser.close" {
                routes.push(&self.browser, Outbound::BrowserClose);
            }
            return;
        }
        if let Some(session) = session.as_deref()
            && !routes.owns(&self.browser, session)
        {
            let error = json!({ "code": SESSION_NOT_FOUND, "message": SESSION_NOT_FOUND_MESSAGE });
            let frame = response_frame(&ResponseFrame {
                id,
                result: None,
                error: Some(&error),
                session_id: Some(session),
            });
            routes.push(&self.browser, Outbound::Frame(frame));
            return;
        }

        let call = Call {
            method,
            params,
            target: session.clone().unwrap_or_else(|| self.browser.clone()),
            context: json!({ "browser": self.browser, "id": id, "session": session }),
        };
        if self.calls.try_send(call).is_err() {
            warn!("downstream client sends too fast; dropping it");
            routes.tombstone(&self.browser);
        }
    }

    pub async fn close(mut self) {
        self.removed = true;
        detach(self.cdp.clone(), self.browser.clone()).await;
    }
}

// Detaching the browser session takes its child sessions with it. The routes go through the
// dispatcher afterwards so frames queued earlier are still discarded, not decoded.
async fn detach(cdp: Cdp, browser: String) {
    let _ = cdp
        .call(
            None,
            "Target.detachFromTarget",
            json!({ "sessionId": browser }),
        )
        .await;
    let _ = cdp.responses.send(browser).await;
}

// A Client that never got going (its upgrade failed) still leaves a session in the browser.
impl Drop for Downstream {
    fn drop(&mut self) {
        if !self.removed
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            runtime.spawn(detach(self.cdp.clone(), self.browser.clone()));
        }
    }
}

fn result_frame(id: u64, result: &Value, session: Option<&str>) -> String {
    response_frame(&ResponseFrame {
        id,
        result: Some(result),
        error: None,
        session_id: session,
    })
}

// A Value always serializes; the empty frame the fallback gives is dropped by the Client.
fn response_frame(frame: &ResponseFrame) -> String {
    serde_json::to_string(frame).unwrap_or_default()
}
