use brr_core::TargetId;
use brr_proto::Incoming;
use serde_json::{Value, json};
use test_case::test_case;

use super::{CdpEvent, TargetInfo, decode};
use crate::SessionId;

const SESSION: &str = "S1";
const TARGET: &str = "T1";
const OPENER: &str = "T0";
const FRAME: &str = "F1";
const LOADER: &str = "L1";

fn session() -> SessionId {
    SessionId(SESSION.to_owned())
}

fn target_info() -> TargetInfo {
    TargetInfo {
        target_id: TargetId(TARGET.to_owned()),
        kind: "page".to_owned(),
        url: "https://example.com/".to_owned(),
        title: "Example".to_owned(),
        opener_id: Some(TargetId(OPENER.to_owned())),
        parent_frame_id: None,
    }
}

fn target_info_json() -> Value {
    json!({
        "targetId": TARGET,
        "type": "page",
        "title": "Example",
        "url": "https://example.com/",
        "attached": false,
        "canAccessOpener": false,
        "openerId": OPENER,
        "browserContextId": "C1"
    })
}

fn incoming(method: &str, params: Value, session_id: Option<&str>) -> Incoming {
    Incoming {
        method: method.to_owned(),
        params,
        session_id: session_id.map(str::to_owned),
    }
}

#[test_case("Target.targetCreated", json!({ "targetInfo": target_info_json() }), None,
    CdpEvent::TargetCreated(target_info()); "target_created")]
#[test_case("Target.targetInfoChanged", json!({ "targetInfo": target_info_json() }), None,
    CdpEvent::TargetInfoChanged(target_info()); "target_info_changed")]
#[test_case("Target.targetDestroyed", json!({ "targetId": TARGET }), None,
    CdpEvent::TargetDestroyed { target_id: TargetId(TARGET.to_owned()) }; "target_destroyed")]
#[test_case("Target.targetCrashed",
    json!({ "targetId": TARGET, "status": "crashed", "errorCode": 11 }), None,
    CdpEvent::TargetCrashed { target_id: TargetId(TARGET.to_owned()) }; "target_crashed")]
#[test_case("Target.attachedToTarget",
    json!({ "sessionId": SESSION, "targetInfo": target_info_json(), "waitingForDebugger": true }),
    None,
    CdpEvent::AttachedToTarget {
        session_id: session(),
        target_info: target_info(),
        waiting_for_debugger: true,
    }; "attached_to_target")]
#[test_case("Target.detachedFromTarget", json!({ "sessionId": SESSION, "targetId": TARGET }), None,
    CdpEvent::DetachedFromTarget { session_id: session() }; "detached_from_target")]
#[test_case("Page.frameStartedLoading", json!({ "frameId": FRAME }), Some(SESSION),
    CdpEvent::FrameStartedLoading { session_id: session(), frame_id: FRAME.to_owned() };
    "frame_started_loading")]
#[test_case("Page.lifecycleEvent",
    json!({ "frameId": FRAME, "loaderId": LOADER, "name": "load", "timestamp": 1.5 }),
    Some(SESSION),
    CdpEvent::LifecycleEvent {
        session_id: session(),
        frame_id: FRAME.to_owned(),
        loader_id: LOADER.to_owned(),
        name: "load".to_owned(),
    }; "lifecycle_event")]
#[test_case("Page.loadEventFired", json!({ "timestamp": 1.5 }), Some(SESSION),
    CdpEvent::LoadEventFired { session_id: session() }; "load_event_fired")]
#[test_case("Network.requestWillBeSent", json!({ "requestId": "R1" }), Some(SESSION),
    CdpEvent::Other {
        method: "Network.requestWillBeSent".to_owned(),
        params: json!({ "requestId": "R1" }),
        session_id: Some(session()),
    }; "unknown_method")]
#[test_case("Page.lifecycleEvent", json!({ "frameId": FRAME }), Some(SESSION),
    CdpEvent::Other {
        method: "Page.lifecycleEvent".to_owned(),
        params: Value::Null,
        session_id: Some(session()),
    }; "malformed_params")]
#[test_case("Page.loadEventFired", json!({}), None,
    CdpEvent::Other {
        method: "Page.loadEventFired".to_owned(),
        params: Value::Null,
        session_id: None,
    }; "page_event_without_session")]
fn decode_event(method: &str, params: Value, session_id: Option<&str>, expected: CdpEvent) {
    assert_eq!(decode(incoming(method, params, session_id)), expected);
}
