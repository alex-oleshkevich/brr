use brr_cdp::SessionId;
use serde_json::{Value, json};

use brr_core::Error;

use super::{HAR_MAX_ENTRIES, Network, Ring, cookie_matches, har_bytes, iso_time};
use brr_core::{Cookie, SameSite};

const URL: &str = "https://example.test/a.js";
const REQUEST_ID: &str = "1.1";
const WALL_TIME: f64 = 1_700_000_000.5;

fn session() -> SessionId {
    SessionId("s1".to_owned())
}

fn started(request_id: &str, redirect: Option<Value>) -> Value {
    let mut params = json!({
        "requestId": request_id,
        "timestamp": 10.0,
        "wallTime": WALL_TIME,
        "type": "Script",
        "request": { "url": URL, "method": "GET", "headers": { "accept": "*/*" } },
    });
    if let Some(redirect) = redirect {
        params["redirectResponse"] = redirect;
    }

    params
}

fn response(status: u16) -> Value {
    json!({
        "status": status,
        "statusText": "OK",
        "headers": { "content-type": "text/javascript" },
        "mimeType": "text/javascript",
        "protocol": "h2",
        "timing": { "requestTime": 10.0, "sendStart": 1.0, "sendEnd": 2.0, "receiveHeadersEnd": 12.0 },
    })
}

fn stop(network: &mut Network) -> Result<Vec<u8>, Error> {
    har_bytes(&network.har_take()?)
}

fn apply(network: &mut Network, method: &str, params: &Value) {
    network.apply(&session(), method, params);
}

fn load(network: &mut Network, request_id: &str) {
    apply(
        network,
        "Network.requestWillBeSent",
        &started(request_id, None),
    );
    apply(
        network,
        "Network.responseReceived",
        &json!({ "requestId": request_id, "response": response(200) }),
    );
    apply(
        network,
        "Network.loadingFinished",
        &json!({ "requestId": request_id, "timestamp": 10.1, "encodedDataLength": 2048.0 }),
    );
}

#[test]
fn ring_keeps_the_newest_in_order() {
    let mut ring = Ring::new(3);
    for item in 0..5 {
        ring.push(item);
    }

    assert_eq!(ring.items, [2, 3, 4]);
}

#[test]
fn request_events_build_one_entry_and_one_har_entry() {
    let mut network = Network::new(10, 10);
    network.har_start().unwrap();
    load(&mut network, REQUEST_ID);

    let log = network.network_log();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].request_id, REQUEST_ID);
    assert_eq!(log[0].url, URL);
    assert_eq!(log[0].status, Some(200));
    assert_eq!(log[0].mime.as_deref(), Some("text/javascript"));
    assert_eq!(log[0].size, Some(2048));
    assert_eq!(log[0].duration_ms, Some(100));

    let har: Value = serde_json::from_slice(&stop(&mut network).unwrap()).unwrap();
    assert_eq!(har["log"]["version"], "1.2");
    let entries = har["log"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["request"]["url"], URL);
    assert_eq!(entries[0]["response"]["status"], 200);
    assert_eq!(entries[0]["response"]["content"]["size"], 2048);
    assert_eq!(entries[0]["startedDateTime"], "2023-11-14T22:13:20.500Z");
}

#[test]
fn har_start_and_stop_reject_wrong_state() {
    let mut network = Network::new(10, 10);
    assert!(stop(&mut network).is_err());

    network.har_start().unwrap();
    assert!(network.har_start().is_err());
    stop(&mut network).unwrap();
    assert!(stop(&mut network).is_err());
}

#[test]
fn har_drops_the_oldest_entries_past_its_cap() {
    let mut network = Network::new(1, 1);
    network.har_start().unwrap();
    for number in 0..=HAR_MAX_ENTRIES {
        apply(
            &mut network,
            "Network.requestWillBeSent",
            &started(&number.to_string(), None),
        );
    }

    let har: Value = serde_json::from_slice(&stop(&mut network).unwrap()).unwrap();
    let entries = har["log"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), HAR_MAX_ENTRIES);
    assert_eq!(network.network_log().len(), 1);
}

#[test]
fn redirect_starts_a_new_entry_and_closes_the_old() {
    let mut network = Network::new(10, 10);
    apply(
        &mut network,
        "Network.requestWillBeSent",
        &started(REQUEST_ID, None),
    );
    apply(
        &mut network,
        "Network.requestWillBeSent",
        &started(REQUEST_ID, Some(response(302))),
    );

    let log = network.network_log();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].status, Some(302));
    assert_eq!(log[1].status, None);
}

#[test]
fn failed_request_records_the_error() {
    let mut network = Network::new(10, 10);
    apply(
        &mut network,
        "Network.requestWillBeSent",
        &started(REQUEST_ID, None),
    );
    apply(
        &mut network,
        "Network.loadingFailed",
        &json!({ "requestId": REQUEST_ID, "timestamp": 10.2, "errorText": "net::ERR_BLOCKED_BY_CLIENT" }),
    );

    assert_eq!(
        network.network_log()[0].failure.as_deref(),
        Some("net::ERR_BLOCKED_BY_CLIENT")
    );
}

#[test]
fn in_flight_requests_gate_idle() {
    let mut network = Network::new(10, 10);
    let quiet = std::time::Duration::ZERO;
    assert!(network.idle(quiet));

    apply(
        &mut network,
        "Network.requestWillBeSent",
        &started(REQUEST_ID, None),
    );
    assert!(!network.idle(quiet));

    network.session_detached(&session());
    assert!(network.idle(quiet));
}

#[test]
fn console_events_become_messages() {
    let mut network = Network::new(10, 10);
    apply(
        &mut network,
        "Runtime.consoleAPICalled",
        &json!({
            "type": "log",
            "args": [{ "type": "string", "value": "a" }, { "type": "number", "value": 2 }, { "type": "object", "description": "Object" }],
            "timestamp": 1000.0,
            "stackTrace": { "callFrames": [{ "url": URL, "lineNumber": 3 }] },
        }),
    );
    apply(
        &mut network,
        "Runtime.exceptionThrown",
        &json!({
            "timestamp": 2000.0,
            "exceptionDetails": { "text": "Uncaught", "exception": { "description": "Error: boom" } },
        }),
    );
    apply(
        &mut network,
        "Log.entryAdded",
        &json!({ "entry": { "level": "warning", "text": "careful", "timestamp": 3000.0 } }),
    );

    let messages = network.console_messages();
    assert_eq!(messages[0].level, "log");
    assert_eq!(messages[0].text, "a 2 Object");
    assert_eq!(messages[0].url.as_deref(), Some(URL));
    assert_eq!(messages[0].line, Some(4));
    assert_eq!(
        (messages[1].level.as_str(), messages[1].text.as_str()),
        ("error", "Error: boom")
    );
    assert_eq!(
        (messages[2].level.as_str(), messages[2].text.as_str()),
        ("warning", "careful")
    );
}

#[test]
fn console_text_is_capped() {
    let mut network = Network::new(10, 10);
    let long = "x".repeat(super::MAX_CONSOLE_TEXT_CHARS + 10);
    apply(
        &mut network,
        "Runtime.consoleAPICalled",
        &json!({ "type": "log", "args": [{ "type": "string", "value": long }], "timestamp": 1.0 }),
    );

    assert_eq!(
        network.console_messages()[0].text.len(),
        super::MAX_CONSOLE_TEXT_CHARS
    );
}

#[test]
fn iso_time_formats_utc() {
    assert_eq!(iso_time(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(iso_time(1_709_251_199_999), "2024-02-29T23:59:59.999Z");
}

fn cookie(domain: &str, path: &str) -> Cookie {
    Cookie {
        name: "k".to_owned(),
        value: "v".to_owned(),
        domain: domain.to_owned(),
        path: path.to_owned(),
        expires: -1.0,
        http_only: false,
        secure: false,
        same_site: SameSite::Lax,
    }
}

#[test]
fn cookies_match_urls_by_domain_and_path() {
    assert!(cookie_matches(
        &cookie("example.test", "/"),
        "https://example.test/a?x=1"
    ));
    assert!(cookie_matches(
        &cookie(".example.test", "/a"),
        "https://www.example.test:8080/a/b"
    ));
    assert!(!cookie_matches(
        &cookie("example.test", "/b"),
        "https://example.test/a"
    ));
    assert!(!cookie_matches(
        &cookie("example.test", "/"),
        "https://badexample.test/"
    ));
    assert!(!cookie_matches(&cookie("example.test", "/"), "not a url"));
}

#[test]
fn urls_are_capped() {
    let mut network = Network::new(10, 10);
    let mut params = started(REQUEST_ID, None);
    params["request"]["url"] = json!("x".repeat(super::MAX_URL_CHARS + 10));
    apply(&mut network, "Network.requestWillBeSent", &params);

    assert_eq!(network.network_log()[0].url.len(), super::MAX_URL_CHARS);
}

#[test]
fn har_wait_is_time_to_first_byte() {
    let mut network = Network::new(10, 10);
    network.har_start().unwrap();
    load(&mut network, REQUEST_ID);

    let har: Value = serde_json::from_slice(&stop(&mut network).unwrap()).unwrap();
    let timings = &har["log"]["entries"][0]["timings"];
    assert_eq!(timings["blocked"], 1.0);
    assert_eq!(timings["send"], 1.0);
    assert_eq!(timings["wait"], 10.0);
}

#[test]
fn redirect_hops_are_timed() {
    let mut network = Network::new(10, 10);
    network.har_start().unwrap();
    apply(
        &mut network,
        "Network.requestWillBeSent",
        &started(REQUEST_ID, None),
    );
    let mut second = started(REQUEST_ID, Some(response(302)));
    second["timestamp"] = json!(10.5);
    apply(&mut network, "Network.requestWillBeSent", &second);

    assert_eq!(network.network_log()[0].duration_ms, Some(500));
    let har: Value = serde_json::from_slice(&stop(&mut network).unwrap()).unwrap();
    assert_eq!(har["log"]["entries"][0]["time"], 500.0);
}
