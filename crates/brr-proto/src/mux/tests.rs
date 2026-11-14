use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::mpsc::Receiver;

use super::Mux;
use crate::{Error, Incoming, MAX_MESSAGE_BYTES};

const PIPE_BYTES: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);
const REMOTE_CODE: i64 = -32000;
const REMOTE_MESSAGE: &str = "No target";
const SMALL_CAP: usize = 64;

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

    async fn send(&mut self, frame: &[u8]) {
        self.writer.write_all(frame).await.unwrap();
        self.writer.write_all(&[0]).await.unwrap();
    }
}

fn connect(max_message_bytes: usize) -> (Mux, Receiver<Incoming>, Peer) {
    let (ours, theirs) = tokio::io::duplex(PIPE_BYTES);
    let (reader, writer) = tokio::io::split(ours);
    let (mux, events) = Mux::start_capped(reader, writer, max_message_bytes);
    let (peer_reader, writer) = tokio::io::split(theirs);
    let peer = Peer {
        reader: BufReader::new(peer_reader),
        writer,
    };

    (mux, events, peer)
}

#[tokio::test]
async fn out_of_order_responses() {
    let (mux, _events, mut peer) = connect(MAX_MESSAGE_BYTES);

    let first = tokio::spawn({
        let mux = mux.clone();
        async move { mux.call("A.first", json!({}), None).await }
    });
    let first_frame = peer.read().await;
    let second = tokio::spawn({
        let mux = mux.clone();
        async move { mux.call("A.second", json!({}), Some("s1")).await }
    });
    let second_frame = peer.read().await;

    assert_eq!(first_frame["id"], 1);
    assert_eq!(first_frame["method"], "A.first");
    assert!(first_frame.get("sessionId").is_none());
    assert_eq!(second_frame["id"], 2);
    assert_eq!(second_frame["sessionId"], "s1");

    peer.send(br#"{"id":2,"result":{"who":"second"}}"#).await;
    peer.send(br#"{"id":1,"result":{"who":"first"}}"#).await;

    let first = tokio::time::timeout(TIMEOUT, first).await.unwrap().unwrap();
    let second = tokio::time::timeout(TIMEOUT, second)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.unwrap(), json!({"who": "first"}));
    assert_eq!(second.unwrap(), json!({"who": "second"}));
}

#[tokio::test]
async fn remote_error() {
    let (mux, _events, mut peer) = connect(MAX_MESSAGE_BYTES);

    let call = tokio::spawn(async move { mux.call("A.b", json!({}), None).await });
    peer.read().await;
    peer.send(
        json!({"id": 1, "error": {"code": REMOTE_CODE, "message": REMOTE_MESSAGE}})
            .to_string()
            .as_bytes(),
    )
    .await;

    let result = tokio::time::timeout(TIMEOUT, call).await.unwrap().unwrap();
    assert!(matches!(
        result,
        Err(Error::Remote { code: REMOTE_CODE, message }) if message == REMOTE_MESSAGE
    ));
}

#[tokio::test]
async fn eof_fails_pending() {
    let (mux, mut events, mut peer) = connect(MAX_MESSAGE_BYTES);

    let call = tokio::spawn({
        let mux = mux.clone();
        async move { mux.call("A.b", json!({}), None).await }
    });
    peer.read().await;
    drop(peer);

    let result = tokio::time::timeout(TIMEOUT, call).await.unwrap().unwrap();
    assert!(matches!(result, Err(Error::Closed)));
    assert!(
        tokio::time::timeout(TIMEOUT, events.recv())
            .await
            .unwrap()
            .is_none()
    );

    let late = mux.call("A.late", json!({}), None).await;
    assert!(matches!(late, Err(Error::Closed)));
}

#[tokio::test]
async fn oversized_frame_skipped() {
    let (_mux, mut events, mut peer) = connect(SMALL_CAP);

    let oversized = json!({"method": "Big.event", "params": {"pad": "x".repeat(SMALL_CAP * 3)}});
    peer.send(oversized.to_string().as_bytes()).await;
    peer.send(br#"{"method":"Small.event","params":{"a":1},"sessionId":"s1"}"#)
        .await;

    let event = tokio::time::timeout(TIMEOUT, events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.method, "Small.event");
    assert_eq!(event.params, json!({"a": 1}));
    assert_eq!(event.session_id.as_deref(), Some("s1"));
}

#[tokio::test]
async fn oversized_frame_over_several_chunks_skipped() {
    let (_mux, mut events, mut peer) = connect(PIPE_BYTES * 2);

    let oversized = json!({"method": "Big.event", "params": {"pad": "x".repeat(PIPE_BYTES * 4)}});
    peer.send(oversized.to_string().as_bytes()).await;
    peer.send(br#"{"method":"Small.event"}"#).await;

    let event = tokio::time::timeout(TIMEOUT, events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.method, "Small.event");
}

#[tokio::test]
async fn malformed_and_unknown_frames_skipped() {
    let (_mux, mut events, mut peer) = connect(MAX_MESSAGE_BYTES);

    peer.send(b"{not json").await;
    peer.send(br#"{"id":99,"result":{}}"#).await;
    peer.send(br#"{"method":"Still.alive"}"#).await;

    let event = tokio::time::timeout(TIMEOUT, events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.method, "Still.alive");
    assert_eq!(event.params, Value::Null);
}

#[tokio::test]
async fn dropped_call_leaves_no_pending_entry() {
    let (mux, _events, mut peer) = connect(MAX_MESSAGE_BYTES);

    let call = tokio::spawn({
        let mux = mux.clone();
        async move { mux.call("A.b", json!({}), None).await }
    });
    peer.read().await;
    call.abort();
    let _ = call.await;

    assert!(mux.shared.lock().as_ref().unwrap().is_empty());
}

#[tokio::test]
async fn forwarded_answer_keeps_stream_order() {
    let (mux, mut events, mut peer) = connect(MAX_MESSAGE_BYTES);

    mux.forward("A.b", json!({}), None, json!({"tag": 7}))
        .await
        .unwrap();
    let request = peer.read().await;
    peer.send(json!({"method": "Early.event"}).to_string().as_bytes())
        .await;
    peer.send(
        json!({"id": request["id"], "result": {"ok": true}})
            .to_string()
            .as_bytes(),
    )
    .await;
    peer.send(json!({"method": "Late.event"}).to_string().as_bytes())
        .await;

    let mut seen = Vec::new();
    for _ in 0..3 {
        seen.push(
            tokio::time::timeout(TIMEOUT, events.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(seen[0].method, "Early.event");
    assert_eq!(seen[1].method, crate::FORWARDED);
    assert_eq!(
        seen[1].params,
        json!({"context": {"tag": 7}, "result": {"ok": true}})
    );
    assert_eq!(seen[2].method, "Late.event");
}

#[tokio::test]
async fn close_cuts_a_write_blocked_on_a_peer_that_stopped_reading() {
    let (mux, _events, _peer) = connect(MAX_MESSAGE_BYTES);
    let call = tokio::spawn({
        let mux = mux.clone();
        async move {
            mux.call("A.big", json!({ "blob": "x".repeat(PIPE_BYTES * 4) }), None)
                .await
        }
    });

    mux.close();

    let result = tokio::time::timeout(TIMEOUT, call).await.unwrap().unwrap();
    assert!(matches!(result, Err(Error::Closed)), "{result:?}");
}
