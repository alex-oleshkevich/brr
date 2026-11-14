use std::path::PathBuf;
use std::time::Duration;

use brr_core::Error;
use brr_proto::{Mux, spawn};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::process::Command;
use tokio::sync::mpsc::Receiver;

use crate::{Cdp, CdpEvent, SessionId};

const PIPE_BYTES: usize = 64 * 1024;
pub(crate) const TIMEOUT: Duration = Duration::from_secs(5);
const SHORT_TIMEOUT: Duration = Duration::from_millis(50);
const SESSION: &str = "S1";
const FRAME: &str = "F1";
const LOADER: &str = "L1";
const CHROME_ENV: &str = "BRR_TEST_CHROME";

pub(crate) struct Peer {
    reader: BufReader<ReadHalf<DuplexStream>>,
    writer: WriteHalf<DuplexStream>,
}

impl Peer {
    pub(crate) async fn read(&mut self) -> Value {
        let mut frame = Vec::new();
        self.reader.read_until(0, &mut frame).await.unwrap();
        frame.pop();
        serde_json::from_slice(&frame).unwrap()
    }

    pub(crate) async fn send(&mut self, frame: Value) {
        self.writer
            .write_all(frame.to_string().as_bytes())
            .await
            .unwrap();
        self.writer.write_all(&[0]).await.unwrap();
    }
}

pub(crate) fn connect(call_timeout: Duration) -> (Cdp, Receiver<CdpEvent>, Peer) {
    let (ours, theirs) = tokio::io::duplex(PIPE_BYTES);
    let (reader, writer) = tokio::io::split(ours);
    let (mux, incoming) = Mux::start(reader, writer);
    let (cdp, events) = Cdp::start_with_timeout(mux, incoming, call_timeout);
    let (peer_reader, writer) = tokio::io::split(theirs);
    let peer = Peer {
        reader: BufReader::new(peer_reader),
        writer,
    };

    (cdp, events, peer)
}

fn session() -> SessionId {
    SessionId(SESSION.to_owned())
}

fn lifecycle(name: &str) -> Value {
    json!({
        "method": "Page.lifecycleEvent",
        "sessionId": SESSION,
        "params": { "frameId": FRAME, "loaderId": LOADER, "name": name, "timestamp": 1.0 },
    })
}

#[tokio::test]
async fn waiter_before_response() {
    let (cdp, _events, mut peer) = connect(TIMEOUT);
    let session = session();

    let waiter = cdp.wait_for(
        Some(&session),
        "Page.lifecycleEvent",
        Box::new(|params| params["name"] == "load"),
    );
    let navigate = tokio::spawn({
        let cdp = cdp.clone();
        let session = session.clone();
        async move { cdp.navigate(&session, "about:blank").await }
    });

    let request = peer.read().await;
    assert_eq!(request["method"], "Page.navigate");
    assert_eq!(request["sessionId"], SESSION);
    peer.send(lifecycle("init")).await;
    peer.send(lifecycle("load")).await;
    peer.send(json!({ "id": request["id"], "result": { "frameId": FRAME, "loaderId": LOADER } }))
        .await;

    let navigated = navigate.await.unwrap().unwrap();
    assert_eq!(navigated.frame_id, FRAME);
    assert_eq!(navigated.loader_id.as_deref(), Some(LOADER));
    let params = tokio::time::timeout(TIMEOUT, waiter)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(params["name"], "load");
    assert_eq!(params["loaderId"], LOADER);
}

#[tokio::test]
async fn dropped_waiter_is_removed() {
    let (cdp, mut events, mut peer) = connect(TIMEOUT);
    let session = session();

    let dropped = cdp.wait_for(Some(&session), "Page.lifecycleEvent", Box::new(|_| false));
    drop(dropped);
    assert_eq!(cdp.waiters.pending(), 1);

    peer.send(lifecycle("load")).await;
    tokio::time::timeout(TIMEOUT, events.recv())
        .await
        .unwrap()
        .unwrap();

    assert!(cdp.waiters.pending() == 0);
}

#[tokio::test]
async fn call_errors() {
    let (cdp, _events, mut peer) = connect(SHORT_TIMEOUT);
    let timed_out = cdp.call(None, "Browser.getVersion", json!({})).await;
    assert_eq!(
        timed_out,
        Err(Error::Timeout {
            after_ms: u64::try_from(SHORT_TIMEOUT.as_millis()).unwrap()
        })
    );
    assert_eq!(peer.read().await["method"], "Browser.getVersion");

    let (cdp, _events, peer) = connect(TIMEOUT);
    drop(peer);
    let closed = cdp.call(None, "Browser.getVersion", json!({})).await;
    assert_eq!(
        closed,
        Err(Error::Internal {
            message: crate::CONNECTION_CLOSED.to_owned()
        })
    );
}

#[tokio::test]
async fn remote_error_names_the_method() {
    let (cdp, _events, mut peer) = connect(TIMEOUT);

    let call = tokio::spawn({
        let cdp = cdp.clone();
        async move { cdp.close_target(&brr_core::TargetId("T9".to_owned())).await }
    });
    let request = peer.read().await;
    peer.send(json!({ "id": request["id"], "error": { "code": -32000, "message": "No target" } }))
        .await;

    assert_eq!(
        call.await.unwrap(),
        Err(Error::Internal {
            message: "Target.closeTarget: No target".to_owned()
        })
    );
}

#[allow(clippy::disallowed_macros)]
#[tokio::test]
async fn browser_version_live() {
    let Some(chrome) = std::env::var_os(CHROME_ENV).map(PathBuf::from) else {
        eprintln!("skipped: {CHROME_ENV} is not set");
        return;
    };
    let profile = tempfile::tempdir().unwrap();

    let mut command = Command::new(chrome);
    command
        .args(["--headless", "--remote-debugging-pipe", "--no-first-run"])
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .kill_on_drop(true);
    let spawned = spawn(command).unwrap();
    let (cdp, _events) = Cdp::start(spawned.mux, spawned.events);

    let product = cdp.browser_version().await.unwrap();

    assert!(!product.is_empty());
}

#[tokio::test]
async fn wait_for_after_close_fails_immediately() {
    let (cdp, mut events, peer) = connect(TIMEOUT);
    drop(peer);
    assert!(events.recv().await.is_none());

    let waiter = cdp.wait_for(None, "Page.loadEventFired", Box::new(|_| true));

    assert!(
        tokio::time::timeout(TIMEOUT, waiter)
            .await
            .unwrap()
            .is_err()
    );
}
