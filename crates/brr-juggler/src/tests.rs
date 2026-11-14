use std::time::Duration;

use brr_core::Error;
use brr_proto::Mux;
use serde_json::{Value, json};
use test_case::test_case;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::mpsc::Receiver;

use crate::{GENERIC_REASON, Juggler, JugglerEvent, MAX_REASON_CHARS, SessionId};

const PIPE_BYTES: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);
const SHORT_TIMEOUT: Duration = Duration::from_millis(50);
const SESSION: &str = "S1";
const FRAME: &str = "F1";
const NAVIGATION: &str = "N1";

struct Peer {
    reader: BufReader<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
}

impl Peer {
    async fn read(&mut self) -> Value {
        let mut frame = Vec::new();
        self.reader.read_until(0, &mut frame).await.unwrap();
        frame.pop();
        serde_json::from_slice(&frame).unwrap()
    }

    async fn send(&mut self, frame: Value) {
        self.writer
            .write_all(frame.to_string().as_bytes())
            .await
            .unwrap();
        self.writer.write_all(&[0]).await.unwrap();
    }
}

fn connect() -> (Juggler, Receiver<JugglerEvent>, Peer) {
    let (ours, theirs) = tokio::io::duplex(PIPE_BYTES);
    let (reader, writer) = tokio::io::split(ours);
    let (mux, incoming) = Mux::start(reader, writer);
    let (juggler, events) = Juggler::start(mux, incoming);
    let (peer_reader, writer) = tokio::io::split(theirs);
    let peer = Peer {
        reader: BufReader::new(peer_reader),
        writer,
    };

    (juggler, events, peer)
}

fn session() -> SessionId {
    SessionId(SESSION.to_owned())
}

#[tokio::test]
async fn waiter_and_errors() {
    let (juggler, _events, mut peer) = connect();
    let load = juggler.wait_for(
        Some(&session()),
        "Page.eventFired",
        Box::new(|params| params["name"] == "load"),
    );

    let navigate = tokio::spawn({
        let juggler = juggler.clone();
        async move {
            juggler
                .navigate(&session(), FRAME, "data:text/html,hi")
                .await
        }
    });
    let request = peer.read().await;
    assert_eq!(request["method"], "Page.navigate");
    assert_eq!(request["sessionId"], SESSION);
    assert_eq!(
        request["params"],
        json!({ "frameId": FRAME, "url": "data:text/html,hi" })
    );

    // The event lands before the response, as Juggler does for fast navigations.
    peer.send(json!({
        "method": "Page.eventFired",
        "params": { "frameId": FRAME, "name": "load" },
        "sessionId": SESSION,
    }))
    .await;
    let fired = tokio::time::timeout(TIMEOUT, load).await.unwrap().unwrap();
    assert_eq!(fired["name"], "load");

    peer.send(json!({ "id": request["id"], "result": { "navigationId": NAVIGATION } }))
        .await;
    let navigated = tokio::time::timeout(TIMEOUT, navigate)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(navigated.navigation_id.as_deref(), Some(NAVIGATION));

    let failing = tokio::spawn({
        let juggler = juggler.clone();
        async move { juggler.navigate(&session(), FRAME, "about:blank").await }
    });
    let request = peer.read().await;
    peer.send(json!({ "id": request["id"], "error": { "message": "No frame" } }))
        .await;
    let error = tokio::time::timeout(TIMEOUT, failing)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    let Error::Internal { message } = error else {
        panic!("expected Internal, got {error:?}");
    };
    assert!(message.contains("Page.navigate: No frame"), "{message}");
}

async fn remote_error(message: &str) -> Error {
    let (juggler, _events, mut peer) = connect();

    let failing =
        tokio::spawn(async move { juggler.navigate(&session(), FRAME, "about:blank").await });
    let request = peer.read().await;
    peer.send(json!({ "id": request["id"], "error": { "message": message } }))
        .await;

    tokio::time::timeout(TIMEOUT, failing)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err()
}

// Juggler's real format: the pretty-printed parameters span several lines, then the check's text.
#[test_case(
    "ERROR: failed to call method 'Page.navigate' with parameters {\n  \"frameId\": 7,\n  \"text\": \"S3cret\"\n}\nExpected \"<root>.frameId\" to be |string|; found |number| `7` instead.",
    "<root>.frameId"; "type mismatch"
)]
#[test_case(
    "ERROR: failed to call method 'Page.navigate' with parameters {\n  \"text\": \"S3cret\"\n}\nFound property \"<root>.text\" - \"S3cret\" which is not described in this scheme",
    "<root>.text"; "undescribed property"
)]
#[test_case(
    "ERROR: failed to call method 'Page.navigate' with parameters {\n  \"text\": \"S3cret\"\n}\nFound object {\n  \"text\": \"S3cret\"\n} which is not described in this scheme",
    "unexpected parameter"; "undescribed root"
)]
#[test_case(
    "ERROR: failed to call method 'Page.navigate' with parameters {\n  \"text\": \"S3cret\"\n}\nsomething unforeseen: S3cret",
    GENERIC_REASON; "unknown detail"
)]
#[tokio::test]
async fn rejected_parameters_are_an_invalid_argument(message: &str, reason: &str) {
    let error = remote_error(message).await;

    let Error::InvalidArgument { field, message } = &error else {
        panic!("expected InvalidArgument, got {error:?}");
    };
    assert_eq!(field, "params");
    assert!(
        message.contains("Page.navigate") && message.contains(reason),
        "{message}"
    );
    assert!(!message.contains("S3cret"), "{message}");
}

#[tokio::test]
async fn rejected_result_is_redacted_and_capped() {
    let long = "x".repeat(MAX_REASON_CHARS * 4);
    let message = format!(
        "ERROR: failed to dispatch method 'Page.navigate' result {{\n  \"cookie\": \"S3cret\"\n}}\nExpected \"<root>.{long}\" to be |string|; found |number| `S3cret` instead."
    );

    let error = remote_error(&message).await;

    let Error::Internal { message } = &error else {
        panic!("expected Internal, got {error:?}");
    };
    assert!(!message.contains("S3cret"), "{message}");
    assert!(message.chars().count() < MAX_REASON_CHARS * 2, "{message}");
}

#[tokio::test]
async fn close_browser_accepts_a_dropped_pipe() {
    let (juggler, _events, mut peer) = connect();

    let close = tokio::spawn(async move { juggler.close_browser().await });
    assert_eq!(peer.read().await["method"], "Browser.close");
    drop(peer);

    tokio::time::timeout(TIMEOUT, close)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn enable_sends_user_prefs_as_name_value_pairs() {
    let (juggler, _events, mut peer) = connect();
    let mut prefs = serde_json::Map::new();
    prefs.insert("a.b".to_owned(), json!(true));

    let enable = tokio::spawn(async move { juggler.enable(&prefs).await });
    let request = peer.read().await;
    assert_eq!(request["method"], "Browser.enable");
    assert_eq!(
        request["params"],
        json!({ "attachToDefaultContext": true, "userPrefs": [{ "name": "a.b", "value": true }] })
    );
    peer.send(json!({ "id": request["id"], "result": {} }))
        .await;

    tokio::time::timeout(TIMEOUT, enable)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn call_within_times_out_with_its_own_limit() {
    let (juggler, _events, mut peer) = connect();

    let timed_out = juggler
        .call_within(None, "Browser.getInfo", json!({}), SHORT_TIMEOUT)
        .await;

    assert_eq!(
        timed_out,
        Err(Error::Timeout {
            after_ms: u64::try_from(SHORT_TIMEOUT.as_millis()).unwrap()
        })
    );
    assert_eq!(peer.read().await["method"], "Browser.getInfo");
}
