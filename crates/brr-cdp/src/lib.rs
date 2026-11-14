mod commands;
mod downstream;
mod events;
mod ws;

use std::time::Duration;

use brr_core::Error;
use brr_proto::{Incoming, Matcher, Mux, Waiters};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

pub use brr_proto::{CONNECTION_CLOSED, SessionId};

pub use commands::NavigateResult;
pub use downstream::{DOWNSTREAM_QUEUE, Downstream, Outbound};
pub use events::{CdpEvent, TargetInfo, decode};
pub use ws::{CONNECT_TIMEOUT, connect_ws, resolve_http_endpoint, start_duplex};

#[cfg(test)]
mod tests;

// CDP answers a parameter it rejects with the JSON-RPC invalid-params code.
const INVALID_PARAMS: i64 = -32602;
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const EVENTS_QUEUE: usize = 1024;

#[derive(Clone)]
pub struct Cdp {
    mux: Mux,
    waiters: Waiters,
    routes: downstream::Routes,
    responses: mpsc::Sender<String>,
    call_timeout: Duration,
}

impl Cdp {
    // The returned receiver MUST be drained in its own task: the dispatcher awaits on it, and a
    // stalled consumer stops waiters from resolving too.
    pub fn start(mux: Mux, incoming: mpsc::Receiver<Incoming>) -> (Cdp, mpsc::Receiver<CdpEvent>) {
        Cdp::start_with_timeout(mux, incoming, CALL_TIMEOUT)
    }

    fn start_with_timeout(
        mux: Mux,
        incoming: mpsc::Receiver<Incoming>,
        call_timeout: Duration,
    ) -> (Cdp, mpsc::Receiver<CdpEvent>) {
        let waiters = Waiters::default();
        let (events_tx, events) = mpsc::channel(EVENTS_QUEUE);

        let routes = downstream::Routes::default();
        let (responses, responses_rx) = mpsc::channel(DOWNSTREAM_QUEUE);

        tokio::spawn(dispatch(
            incoming,
            waiters.clone(),
            routes.clone(),
            responses_rx,
            events_tx,
        ));

        (
            Cdp {
                mux,
                waiters,
                routes,
                responses,
                call_timeout,
            },
            events,
        )
    }

    pub fn close(&self) {
        self.mux.close();
    }

    pub async fn call(
        &self,
        session_id: Option<&SessionId>,
        method: &str,
        params: Value,
    ) -> Result<Value, Error> {
        self.call_within(session_id, method, params, self.call_timeout)
            .await
    }

    // For the few commands that legitimately run long, like printing a large document.
    pub async fn call_within(
        &self,
        session_id: Option<&SessionId>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, Error> {
        self.mux
            .call_within(
                session_id.map(|session| session.0.as_str()),
                method,
                params,
                timeout,
                remote_error,
            )
            .await
    }

    pub fn wait_for(
        &self,
        session_id: Option<&SessionId>,
        method: &'static str,
        matches: Matcher,
    ) -> oneshot::Receiver<Value> {
        self.waiters.wait_for(session_id, method, matches)
    }
}

fn remote_error(method: &str, code: i64, message: &str) -> Error {
    if code == INVALID_PARAMS {
        Error::InvalidArgument {
            field: "params".to_owned(),
            message: format!("{method}: {message}"),
        }
    } else {
        Error::Internal {
            message: format!("{method}: {message}"),
        }
    }
}

async fn dispatch(
    mut incoming: mpsc::Receiver<Incoming>,
    waiters: Waiters,
    routes: downstream::Routes,
    mut responses: mpsc::Receiver<String>,
    events: mpsc::Sender<CdpEvent>,
) {
    loop {
        let frame = tokio::select! {
            biased;
            frame = incoming.recv() => match frame {
                Some(frame) => frame,
                None => break,
            },
            Some(browser) = responses.recv() => {
                routes.remove_client(&browser);
                continue;
            }
        };
        let Some(frame) = downstream::route(&routes, frame) else {
            continue;
        };
        waiters.resolve(&frame);

        // A dropped receiver only means nobody wants events; waiters must keep resolving.
        let _ = events.send(decode(frame)).await;
    }

    // Ends every Client's receiver now that the browser is gone.
    routes.clear();

    waiters.close();
}
