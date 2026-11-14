mod commands;
mod events;

use std::time::Duration;

use brr_core::Error;
use brr_proto::{Incoming, Matcher, Mux, Waiters};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

pub use brr_proto::SessionId;

pub use commands::NavigateResult;
pub use events::{JugglerEvent, decode};

#[cfg(test)]
mod tests;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const EVENTS_QUEUE: usize = 1024;
// Juggler has no error codes; its parameter check fails with this text, followed by the details.
const BAD_PARAMS_MARKER: &str = "failed to call method";
const BAD_RESULT_MARKER: &str = "failed to dispatch method";
const DETAIL_STARTS: [&str; 3] = ["Expected \"", "Object \"", UNDESCRIBED_START];
const UNDESCRIBED_START: &str = "Found ";
const GENERIC_REASON: &str = "the browser rejected the call's shape";
const UNEXPECTED_PARAMETER: &str = "unexpected parameter";
const MAX_REASON_CHARS: usize = 200;

#[derive(Clone)]
pub struct Juggler {
    mux: Mux,
    waiters: Waiters,
}

impl Juggler {
    // The returned receiver MUST be drained in its own task, and a call MUST NOT be awaited in
    // the loop that drains it: the dispatcher awaits on the receiver, so a stalled consumer stops
    // waiters and responses from resolving too.
    pub fn start(
        mux: Mux,
        incoming: mpsc::Receiver<Incoming>,
    ) -> (Juggler, mpsc::Receiver<JugglerEvent>) {
        let waiters = Waiters::default();
        let (events_tx, events) = mpsc::channel(EVENTS_QUEUE);

        tokio::spawn(dispatch(incoming, waiters.clone(), events_tx));

        (Juggler { mux, waiters }, events)
    }

    pub async fn call(
        &self,
        session_id: Option<&SessionId>,
        method: &str,
        params: Value,
    ) -> Result<Value, Error> {
        self.call_within(session_id, method, params, CALL_TIMEOUT)
            .await
    }

    // For the few commands that legitimately run long.
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

fn remote_error(method: &str, _code: i64, message: &str) -> Error {
    if message.contains(BAD_PARAMS_MARKER) {
        Error::InvalidArgument {
            field: "params".to_owned(),
            message: format!("{method}: {}", rejection_reason(message)),
        }
    } else if message.contains(BAD_RESULT_MARKER) {
        Error::Internal {
            message: format!(
                "{method}: the browser returned an unexpected result: {}",
                rejection_reason(message)
            ),
        }
    } else {
        Error::Internal {
            message: format!("{method}: {message}"),
        }
    }
}

// Juggler's checker fails with `ERROR: failed to call method '<m>' with parameters <pretty JSON>`
// and then the check's own text; both echo the call's values, which can be typed secrets or page
// content. The JSON lines are indented or start with a bracket, so only an unindented line that
// begins like a checker message is trusted, and only its value-free head is kept.
fn rejection_reason(message: &str) -> String {
    let Some(line) = message
        .lines()
        .find(|line| DETAIL_STARTS.iter().any(|start| line.starts_with(start)))
    else {
        return GENERIC_REASON.to_owned();
    };

    let reason = if line.starts_with(UNDESCRIBED_START) {
        // The property's value follows its quoted path.
        match line.split('"').nth(1) {
            Some(path) => format!("unexpected parameter {path}"),
            None => UNEXPECTED_PARAMETER.to_owned(),
        }
    } else {
        // Every "Expected ..." message puts the offending value after a semicolon.
        line.split(';').next().unwrap_or_default().to_owned()
    };

    reason.chars().take(MAX_REASON_CHARS).collect()
}

async fn dispatch(
    mut incoming: mpsc::Receiver<Incoming>,
    waiters: Waiters,
    events: mpsc::Sender<JugglerEvent>,
) {
    while let Some(frame) = incoming.recv().await {
        waiters.resolve(&frame);

        // A dropped receiver only means nobody wants events; waiters must keep resolving.
        let _ = events.send(decode(frame)).await;
    }

    waiters.close();
}
