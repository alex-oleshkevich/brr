use brr_core::TargetId;
use brr_proto::Incoming;
use serde_json::{Value, json};
use test_case::test_case;

use super::{JugglerEvent, decode};
use crate::SessionId;

const SESSION: &str = "S1";
const TARGET: &str = "T1";
const OPENER: &str = "T0";
const CONTEXT: &str = "C1";
const UUID: &str = "D1";
const URL: &str = "https://example.com/file.bin";
const FILE: &str = "file.bin";
const FRAME: &str = "F1";

fn incoming(method: &str, params: Value, session_id: Option<&str>) -> Incoming {
    Incoming {
        method: method.to_owned(),
        params,
        session_id: session_id.map(str::to_owned),
    }
}

fn target() -> Value {
    json!({
        "sessionId": SESSION,
        "targetInfo": {
            "type": "page",
            "targetId": TARGET,
            "browserContextId": CONTEXT,
            "openerId": OPENER,
        },
    })
}

fn other(method: &str, params: Value, session_id: Option<&str>) -> JugglerEvent {
    JugglerEvent::Other {
        method: method.to_owned(),
        params,
        session_id: session_id.map(|id| SessionId(id.to_owned())),
    }
}

fn download_created() -> Value {
    json!({
        "uuid": UUID,
        "pageTargetId": TARGET,
        "frameId": FRAME,
        "url": URL,
        "suggestedFileName": FILE,
    })
}

#[test_case(
    "Browser.attachedToTarget", target(), None,
    JugglerEvent::AttachedToTarget {
        session_id: SessionId(SESSION.to_owned()),
        target_id: TargetId(TARGET.to_owned()),
        browser_context_id: Some(CONTEXT.to_owned()),
        opener_id: Some(TargetId(OPENER.to_owned())),
    };
    "attached page"
)]
#[test_case(
    "Browser.detachedFromTarget", json!({ "sessionId": SESSION, "targetId": TARGET }), None,
    JugglerEvent::DetachedFromTarget {
        session_id: SessionId(SESSION.to_owned()),
        target_id: TargetId(TARGET.to_owned()),
    };
    "detached"
)]
#[test_case(
    "Browser.downloadCreated", download_created(), None,
    JugglerEvent::DownloadCreated {
        uuid: UUID.to_owned(),
        page_target_id: TargetId(TARGET.to_owned()),
        url: URL.to_owned(),
        suggested_file_name: FILE.to_owned(),
    };
    "download created"
)]
#[test_case(
    "Browser.downloadFinished", json!({ "uuid": UUID, "canceled": true, "error": "boom" }), None,
    JugglerEvent::DownloadFinished {
        uuid: UUID.to_owned(),
        canceled: true,
        error: Some("boom".to_owned()),
    };
    "download finished"
)]
#[test_case(
    "Browser.downloadFinished", json!({ "uuid": UUID }), None,
    JugglerEvent::DownloadFinished { uuid: UUID.to_owned(), canceled: false, error: None };
    "download finished defaults"
)]
#[test_case(
    "Page.eventFired", json!({ "frameId": FRAME, "name": "load" }), Some(SESSION),
    other("Page.eventFired", json!({ "frameId": FRAME, "name": "load" }), Some(SESSION));
    "page event"
)]
#[test_case(
    "Browser.detachedFromTarget", json!({ "nope": 1 }), None,
    other("Browser.detachedFromTarget", Value::Null, None);
    "malformed"
)]
fn decodes(method: &str, params: Value, session_id: Option<&str>, expected: JugglerEvent) {
    assert_eq!(decode(incoming(method, params, session_id)), expected);
}
