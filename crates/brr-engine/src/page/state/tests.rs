use std::borrow::Cow;
use std::time::Duration;

use serde_json::json;

use brr_cdp::SessionId;
use brr_core::Error;
use brr_core::capture::ResourceType;
use tokio::time::Instant;

use crate::chromium::fetch::RequestRules;

use super::PageState;

const SECRET: &str = "S3cret!";
const NETWORK_WAIT: Duration = Duration::from_millis(100);
const REDACTED: &str = "«redacted»";

fn state() -> PageState {
    PageState::new(10, 10)
}

#[test]
fn redact_replaces_every_secret_longest_first() {
    let state = state();
    state.add_secret(SECRET);
    state.add_secret("S3cret!2");

    assert_eq!(
        state.redact("a S3cret! b S3cret!2 c"),
        format!("a {REDACTED} b {REDACTED} c")
    );
}

#[test]
fn redact_borrows_when_nothing_matches() {
    let state = state();
    assert!(matches!(state.redact("nothing"), Cow::Borrowed(_)));

    state.add_secret(SECRET);
    assert!(matches!(state.redact("nothing"), Cow::Borrowed(_)));
}

#[test]
fn redact_does_not_rescan_its_replacement() {
    let state = state();
    state.add_secret("act");

    assert_eq!(state.redact("an act"), format!("an {REDACTED}"));
}

#[test]
fn redact_ignores_an_empty_secret() {
    let state = state();
    state.add_secret("");

    assert!(matches!(state.redact("text"), Cow::Borrowed(_)));
}

#[test]
fn redact_value_rewrites_nested_strings() {
    let state = state();
    state.add_secret(SECRET);
    let mut value = json!({ "a": [SECRET, 1, { "b": format!("x{SECRET}") }], "c": null });

    state.redact_value(&mut value);

    assert_eq!(
        value,
        json!({ "a": [REDACTED, 1, { "b": format!("x{REDACTED}") }], "c": null })
    );
}

#[test]
fn redact_catches_the_forms_a_page_or_url_shows() {
    let state = state();
    state.add_secret(r#"a{"b"\c&d !"#);

    for shown in [
        r#"a{\"b\"\\c&d !"#,
        "a{&quot;b&quot;\\c&amp;d !",
        "a%7B%22b%22%5Cc%26d%20%21",
        "a%7B%22b%22%5Cc%26d+%21",
        "a%7B%22b%22%5Cc%26d%20!",
    ] {
        assert_eq!(state.redact(shown), REDACTED, "{shown}");
    }
}

#[test]
fn redact_value_rewrites_keys() {
    let state = state();
    state.add_secret(SECRET);
    let mut value = json!({ SECRET: 1 });

    state.redact_value(&mut value);

    assert_eq!(value, json!({ REDACTED: 1 }));
}

#[test]
fn a_secret_stays_on_the_page_until_the_main_frame_navigates() {
    let state = state();
    let session = SessionId("s1".to_owned());
    state.apply(
        &session,
        "Page.frameNavigated",
        &json!({ "frame": { "id": "main", "url": "https://a.test/" } }),
    );
    assert!(!state.holds_secret());

    state.add_secret(SECRET);
    state.apply(
        &session,
        "Page.frameNavigated",
        &json!({ "frame": { "id": "child", "parentId": "main", "url": "https://a.test/f" } }),
    );
    assert!(state.holds_secret());

    state.apply(
        &session,
        "Page.frameNavigated",
        &json!({ "frame": { "id": "main", "url": "https://a.test/next" } }),
    );
    assert!(!state.holds_secret());
}

#[test]
fn main_frame_navigations_are_counted() {
    let state = state();
    let session = SessionId("s1".to_owned());
    let count = |state: &PageState| *state.navigations().borrow();
    let frame = |parent: Option<&str>| json!({ "frame": { "id": "f", "parentId": parent, "url": "https://a.test/" } });

    state.apply(&session, "Page.frameNavigated", &frame(None));
    state.apply(&session, "Page.frameNavigated", &frame(Some("main")));
    assert_eq!(count(&state), 1);

    state.apply(&session, "Page.frameNavigated", &frame(None));
    assert_eq!(count(&state), 2);
}

#[test]
fn type_only_rules_pause_only_those_types() {
    let state = state();
    state.set_rules(RequestRules {
        patterns: Vec::new(),
        types: vec![ResourceType::Image, ResourceType::Xhr],
    });

    assert_eq!(
        state.fetch_patterns(),
        json!([
            { "resourceType": "Image", "requestStage": "Request" },
            { "resourceType": "XHR", "requestStage": "Request" },
            { "resourceType": "Fetch", "requestStage": "Request" },
        ])
    );
}

#[test]
fn a_url_rule_pauses_every_request() {
    let state = state();
    state.set_rules(RequestRules {
        patterns: vec!["*.png".to_owned()],
        types: vec![ResourceType::Image],
    });

    assert_eq!(
        state.fetch_patterns(),
        json!([{ "urlPattern": "*", "requestStage": "Request" }])
    );
}

#[tokio::test]
async fn waiting_for_idle_times_out_while_a_request_is_in_flight() {
    let state = state();
    state.add_session(&SessionId("s1".to_owned()));
    state.apply(
        &SessionId("s1".to_owned()),
        "Network.requestWillBeSent",
        &json!({
            "requestId": "1.1",
            "timestamp": 10.0,
            "wallTime": 1_700_000_000.5,
            "type": "XHR",
            "request": { "url": "https://example.test/", "method": "GET", "headers": {} },
        }),
    );

    let waited = state.wait_network_idle(Instant::now(), NETWORK_WAIT).await;

    assert_eq!(
        waited,
        Err(Error::Timeout {
            after_ms: NETWORK_WAIT.as_millis() as u64
        })
    );
}
