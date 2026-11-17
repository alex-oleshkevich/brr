use std::time::Duration;

use bytes::{Bytes, BytesMut};
use test_case::test_case;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

use super::{
    Frame, FrameError, FrameKind, Link, MAX_PAYLOAD, PIPE, STREAM_WINDOW, Side, Stream, read_frame,
    write_frame,
};

const TRANSPORT_BYTES: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);
// Long enough for a writer that could still make progress to make it.
const SETTLE: Duration = Duration::from_millis(200);
const ROUNDS: usize = 50;
const PIPE_TOTAL: usize = 4 * STREAM_WINDOW;

struct End {
    link: Link,
    pipe: Stream,
    accepted: Receiver<Stream>,
}

fn pair() -> (End, End) {
    let (daemon, runner) = tokio::io::duplex(TRANSPORT_BYTES);
    (start(daemon, Side::Daemon), start(runner, Side::Runner))
}

fn start(transport: DuplexStream, side: Side) -> End {
    let (reader, writer) = tokio::io::split(transport);
    let (link, pipe, accepted) = Link::start(reader, writer, side);

    End {
        link,
        pipe,
        accepted,
    }
}

fn frame(stream: u32, kind: FrameKind, payload: &[u8]) -> Frame {
    Frame {
        stream,
        kind,
        payload: Bytes::copy_from_slice(payload),
    }
}

async fn encoded(frame: &Frame) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, frame).await.unwrap();
    bytes
}

async fn read_all(stream: &mut Stream) -> Vec<u8> {
    let mut read = Vec::new();
    timeout(TIMEOUT, stream.read_to_end(&mut read))
        .await
        .unwrap()
        .unwrap();
    read
}

#[test_case(frame(PIPE, FrameKind::Data, b"{\"id\":1}\0") ; "pipe data")]
#[test_case(frame(1, FrameKind::Open, b"") ; "tunnel open")]
#[test_case(frame(2, FrameKind::Open, b"{\"put\":\"/data\"}") ; "file open")]
#[test_case(frame(3, FrameKind::Close, b"") ; "close")]
#[test_case(frame(u32::MAX, FrameKind::Window, &[0, 4, 0, 0]) ; "window")]
#[test_case(frame(5, FrameKind::Data, &[7; MAX_PAYLOAD]) ; "largest payload")]
#[tokio::test]
async fn frame_round_trips(frame: Frame) {
    let bytes = encoded(&frame).await;
    let mut reader = bytes.as_slice();
    let mut buffer = BytesMut::new();

    let read = read_frame(&mut reader, &mut buffer).await.unwrap();

    assert_eq!(read, Some(frame));
    assert_eq!(read_frame(&mut reader, &mut buffer).await.unwrap(), None);
}

#[test_case(&[0, 0, 0, 1, 9, 0, 0, 0, 0] ; "unknown kind")]
#[test_case(&[0, 0, 0, 1, 2, 0, 1, 0, 1] ; "payload over the cap")]
#[test_case(&[0, 0, 0, 1, 4, 0, 0, 0, 2, 0, 1] ; "short window")]
#[test_case(&[0, 0, 0, 1, 2] ; "truncated header")]
#[test_case(&[0, 0, 0, 1, 2, 0, 0, 0, 4, 1, 2] ; "truncated payload")]
#[tokio::test]
async fn malformed_frame_fails(bytes: &[u8]) {
    let mut reader = bytes;

    let read = read_frame(&mut reader, &mut BytesMut::new()).await;

    assert!(
        matches!(
            read,
            Err(FrameError::UnknownKind(_)
                | FrameError::TooLong(_)
                | FrameError::BadWindow(_)
                | FrameError::Truncated)
        ),
        "{read:?}"
    );
}

#[tokio::test]
async fn streams_arrive_intact_and_in_order() {
    let (mut daemon, runner) = pair();
    let mut first = runner.link.open(b"").unwrap();
    let mut second = runner.link.open(b"").unwrap();
    let mut runner_pipe = runner.pipe;

    let writer = tokio::spawn(async move {
        for round in 0..ROUNDS {
            runner_pipe
                .write_all(format!("pipe {round};").as_bytes())
                .await
                .unwrap();
            first
                .write_all(format!("first {round};").as_bytes())
                .await
                .unwrap();
            second
                .write_all(format!("second {round};").as_bytes())
                .await
                .unwrap();
        }
        for stream in [&mut runner_pipe, &mut first, &mut second] {
            stream.shutdown().await.unwrap();
        }
    });
    let mut accepted_first = daemon.accepted.recv().await.unwrap();
    let mut accepted_second = daemon.accepted.recv().await.unwrap();
    let (pipe, first, second) = tokio::join!(
        read_all(&mut daemon.pipe),
        read_all(&mut accepted_first),
        read_all(&mut accepted_second)
    );
    writer.await.unwrap();

    let expected = |name: &str| -> Vec<u8> {
        (0..ROUNDS)
            .map(|round| format!("{name} {round};"))
            .collect::<String>()
            .into_bytes()
    };
    assert_eq!(accepted_first.id(), 1);
    assert_eq!(accepted_second.id(), 3);
    assert_eq!(pipe, expected("pipe"));
    assert_eq!(first, expected("first"));
    assert_eq!(second, expected("second"));
}

#[tokio::test]
async fn a_stalled_stream_does_not_block_the_pipe() {
    let (mut daemon, runner) = pair();
    let mut stalled = runner.link.open(b"").unwrap();
    let mut runner_pipe = runner.pipe;
    // Accepted and never read.
    let _unread = daemon.accepted.recv().await.unwrap();

    let flooding = tokio::spawn(async move {
        let chunk = vec![1; MAX_PAYLOAD];
        let mut written = 0;
        loop {
            stalled.write_all(&chunk).await.unwrap();
            written += chunk.len();
            if written > 2 * STREAM_WINDOW {
                return written;
            }
        }
    });
    let piping = tokio::spawn(async move {
        runner_pipe.write_all(&vec![2; PIPE_TOTAL]).await.unwrap();
        runner_pipe.shutdown().await.unwrap();
    });

    let pipe = read_all(&mut daemon.pipe).await;
    piping.await.unwrap();
    tokio::time::sleep(SETTLE).await;

    assert_eq!(pipe.len(), PIPE_TOTAL);
    assert!(
        !flooding.is_finished(),
        "the stalled stream wrote past its window"
    );
    flooding.abort();
}

#[tokio::test]
async fn close_ends_one_direction() {
    let (mut daemon, runner) = pair();
    let mut tunnel = runner.link.open(b"").unwrap();
    tunnel.write_all(b"request").await.unwrap();
    tunnel.shutdown().await.unwrap();
    let mut accepted = daemon.accepted.recv().await.unwrap();

    assert_eq!(read_all(&mut accepted).await, b"request");
    accepted.write_all(b"response").await.unwrap();
    drop(accepted);
    assert_eq!(read_all(&mut tunnel).await, b"response");
}

#[tokio::test]
async fn an_open_nobody_takes_is_closed() {
    let (daemon, runner) = pair();
    drop(runner.accepted);
    let mut transfer = daemon.link.open(b"{\"put\":\"/data\"}").unwrap();

    assert_eq!(transfer.id(), 2);
    assert_eq!(read_all(&mut transfer).await, b"");
}

#[tokio::test]
async fn transport_eof_ends_every_stream() {
    let (ours, theirs) = tokio::io::duplex(TRANSPORT_BYTES);
    let mut runner = start(ours, Side::Runner);
    let mut tunnel = runner.link.open(b"").unwrap();
    drop(theirs);

    assert_eq!(read_all(&mut runner.pipe).await, b"");
    assert_eq!(read_all(&mut tunnel).await, b"");
    assert!(runner.accepted.recv().await.is_none());
    assert!(tunnel.write_all(b"late").await.is_err());
}

#[tokio::test]
async fn an_open_of_our_parity_ends_the_link() {
    let (ours, mut theirs) = tokio::io::duplex(TRANSPORT_BYTES);
    let mut daemon = start(ours, Side::Daemon);

    let bytes = encoded(&frame(2, FrameKind::Open, b"")).await;
    theirs.write_all(&bytes).await.unwrap();

    assert_eq!(read_all(&mut daemon.pipe).await, b"");
}
