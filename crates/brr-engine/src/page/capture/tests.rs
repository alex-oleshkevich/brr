#![allow(clippy::unwrap_used)]

use brr_cdp::{Cdp, SessionId};
use brr_core::Error;
use brr_proto::Mux;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use test_case::test_case;

use super::{clip_json, print_error, read_stream};

const LIMIT: usize = 8;
const PIPE_BYTES: usize = 64 * 1024;

// A browser that answers IO.read with the given chunks, one per call.
fn serve(chunks: Vec<Value>) -> Cdp {
    let (ours, theirs) = tokio::io::duplex(PIPE_BYTES);
    let (reader, writer) = tokio::io::split(ours);
    let (mux, incoming) = Mux::start(reader, writer);
    let (cdp, _events) = Cdp::start(mux, incoming);

    tokio::spawn(async move {
        let (peer_reader, mut peer_writer) = tokio::io::split(theirs);
        let mut peer_reader = BufReader::new(peer_reader);
        for chunk in chunks {
            let mut frame = Vec::new();
            if peer_reader.read_until(0, &mut frame).await.unwrap_or(0) == 0 {
                return;
            }
            frame.pop();
            let request: Value = serde_json::from_slice(&frame).unwrap();
            let reply = json!({ "id": request["id"], "result": chunk });
            peer_writer
                .write_all(reply.to_string().as_bytes())
                .await
                .unwrap();
            peer_writer.write_all(&[0]).await.unwrap();
        }
    });

    cdp
}

fn session() -> SessionId {
    SessionId("s1".to_owned())
}

#[tokio::test]
async fn a_stream_within_the_limit_is_read_whole() {
    let cdp = serve(vec![
        json!({ "data": "abcd", "base64Encoded": false, "eof": false }),
        json!({ "data": "efgh", "base64Encoded": false, "eof": true }),
    ]);

    let pdf = read_stream(&cdp, &session(), "h", LIMIT).await.unwrap();

    assert_eq!(pdf, b"abcdefgh");
}

#[tokio::test]
async fn a_stream_past_the_limit_is_refused() {
    let cdp = serve(vec![
        json!({ "data": "abcdef", "base64Encoded": false, "eof": false }),
        json!({ "data": "ghi", "base64Encoded": false, "eof": true }),
    ]);

    let error = read_stream(&cdp, &session(), "h", LIMIT).await.unwrap_err();

    assert!(matches!(error, Error::InvalidArgument { field, .. } if field == "area"));
}

#[tokio::test]
async fn an_encoded_chunk_counts_its_decoded_size() {
    // Twelve base64 characters decode to nine bytes.
    let cdp = serve(vec![
        json!({ "data": "QUJDREVGR0hJ", "base64Encoded": true, "eof": true }),
    ]);

    let error = read_stream(&cdp, &session(), "h", LIMIT).await.unwrap_err();

    assert!(matches!(error, Error::InvalidArgument { .. }));
}

#[test]
fn a_print_failure_names_the_way_out() {
    let error = print_error(Error::Internal {
        message: "Page.printToPDF: Printing failed".to_owned(),
    });

    assert!(matches!(error, Error::InvalidArgument { field, message }
            if field == "pages" && message.contains("page_ranges")));
}

#[test]
fn other_errors_pass_through() {
    let error = print_error(Error::Internal {
        message: "closed".to_owned(),
    });

    assert!(matches!(error, Error::Internal { message } if message == "closed"));
}

const CSS_SIDE: f64 = 5000.0;

#[test_case(1.0, true; "css pixels fit")]
#[test_case(3.0, true; "triple scale fits the side limit")]
#[test_case(4.0, false; "quadruple scale passes the side limit")]
fn the_scale_counts_toward_the_side_limit(scale: f64, fits: bool) {
    assert_eq!(clip_json(0.0, 0.0, CSS_SIDE, 10.0, scale).is_ok(), fits);
}

#[test]
fn base64_expansion_counts_toward_the_byte_limit() {
    // 8192 x 8192 x 4 bytes is 256 MiB raw: exactly the cap before encoding, over it after.
    let side = 8192.0;

    assert!(clip_json(0.0, 0.0, side, side, 1.0).is_err());
    assert!(clip_json(0.0, 0.0, 4096.0, 4096.0, 1.0).is_ok());
}
