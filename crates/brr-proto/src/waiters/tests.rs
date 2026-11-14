use serde_json::json;

use super::{SessionId, Waiters};
use crate::Incoming;

const SESSION: &str = "S1";
const METHOD: &str = "Page.eventFired";

fn frame(session: Option<&str>, name: &str) -> Incoming {
    Incoming {
        method: METHOD.to_owned(),
        params: json!({ "name": name }),
        session_id: session.map(str::to_owned),
    }
}

fn session() -> SessionId {
    SessionId(SESSION.to_owned())
}

#[test]
fn a_waiter_resolves_on_its_session_and_matcher_only() {
    let waiters = Waiters::default();
    let mut load = waiters.wait_for(
        Some(&session()),
        METHOD,
        Box::new(|params| params["name"] == "load"),
    );

    waiters.resolve(&frame(Some(SESSION), "other"));
    waiters.resolve(&frame(None, "load"));
    assert!(load.try_recv().is_err());

    assert_eq!(waiters.pending(), 1);
    waiters.resolve(&frame(Some(SESSION), "load"));
    assert_eq!(waiters.pending(), 0);
    assert_eq!(load.try_recv().unwrap(), json!({ "name": "load" }));
}

#[test]
fn closing_fails_pending_and_later_waiters() {
    let waiters = Waiters::default();
    let mut pending = waiters.wait_for(None, METHOD, Box::new(|_| true));

    waiters.close();
    let mut late = waiters.wait_for(None, METHOD, Box::new(|_| true));

    assert!(pending.try_recv().is_err());
    assert!(late.try_recv().is_err());
    assert!(matches!(
        pending.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
    assert!(matches!(
        late.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Closed)
    ));
}
