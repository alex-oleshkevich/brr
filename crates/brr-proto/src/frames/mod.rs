use std::collections::{HashMap, VecDeque};
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use bytes::{Buf, Bytes, BytesMut};
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadBuf,
};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

#[cfg(test)]
mod tests;

// The browser pipe: exists from the start on both sides, never opened.
pub const PIPE: u32 = 0;
pub const MAX_PAYLOAD: usize = 64 * 1024;
// Bytes a sender may have unacknowledged on one stream. The receiver buffers at most this much
// per stream and grants more with Window frames as its reader consumes, so a stream nobody reads
// stalls only its own sender and never the connection.
pub const STREAM_WINDOW: usize = 1024 * 1024;

const HEADER_BYTES: usize = 9;
const WINDOW_BYTES: usize = 4;
// Consumed bytes are granted back in batches rather than per read.
const CREDIT_BATCH: usize = STREAM_WINDOW / 4;
// Streams the peer opened that the owner has not taken yet; more are refused with Close.
const ACCEPT_QUEUE: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
    // Payload: the opener's request, empty for a proxy tunnel.
    Open = 1,
    Data = 2,
    // The sender's direction ended, like a TCP FIN; the other direction stays open.
    Close = 3,
    // Payload: u32 BE count of bytes the receiver consumed, which the sender may send again.
    Window = 4,
}

impl TryFrom<u8> for FrameKind {
    type Error = FrameError;

    fn try_from(byte: u8) -> Result<Self, FrameError> {
        match byte {
            1 => Ok(FrameKind::Open),
            2 => Ok(FrameKind::Data),
            3 => Ok(FrameKind::Close),
            4 => Ok(FrameKind::Window),
            other => Err(FrameError::UnknownKind(other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub stream: u32,
    pub kind: FrameKind,
    pub payload: Bytes,
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("unknown frame kind {0}")]
    UnknownKind(u8),
    #[error("frame payload of {0} bytes is over the {MAX_PAYLOAD} byte cap")]
    TooLong(u32),
    #[error("window frame payload of {0} bytes is not {WINDOW_BYTES}")]
    BadWindow(u32),
    #[error("the connection ended inside a frame")]
    Truncated,
    #[error(transparent)]
    Io(#[from] io::Error),
}

// None when the connection ended cleanly between frames.
pub async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    buffer: &mut BytesMut,
) -> Result<Option<Frame>, FrameError> {
    let mut header = [0; HEADER_BYTES];
    let first = reader.read(&mut header).await?;
    if first == 0 {
        return Ok(None);
    }
    reader
        .read_exact(&mut header[first..])
        .await
        .map_err(truncated)?;

    let stream = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    let kind = FrameKind::try_from(header[4])?;
    let len = u32::from_be_bytes([header[5], header[6], header[7], header[8]]);
    if len as usize > MAX_PAYLOAD {
        return Err(FrameError::TooLong(len));
    }
    if kind == FrameKind::Window && len as usize != WINDOW_BYTES {
        return Err(FrameError::BadWindow(len));
    }

    buffer.resize(len as usize, 0);
    reader.read_exact(buffer).await.map_err(truncated)?;

    Ok(Some(Frame {
        stream,
        kind,
        payload: buffer.split().freeze(),
    }))
}

fn truncated(error: io::Error) -> FrameError {
    match error.kind() {
        io::ErrorKind::UnexpectedEof => FrameError::Truncated,
        _ => FrameError::Io(error),
    }
}

// Callers keep payloads within MAX_PAYLOAD; the peer rejects a longer frame.
pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &Frame) -> io::Result<()> {
    let mut header = [0; HEADER_BYTES];
    header[..4].copy_from_slice(&frame.stream.to_be_bytes());
    header[4] = frame.kind as u8;
    header[5..].copy_from_slice(&(frame.payload.len() as u32).to_be_bytes());

    writer.write_all(&header).await?;
    writer.write_all(&frame.payload).await
}

// Which end of the connection this is; each side opens streams of its own parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    // Opens even streams: file transfers.
    Daemon,
    // Opens odd streams: proxy tunnels.
    Runner,
}

impl Side {
    fn first_stream(self) -> u32 {
        match self {
            Side::Daemon => 2,
            Side::Runner => 1,
        }
    }
}

enum Out {
    Frame(Frame),
    Flushed(oneshot::Sender<()>),
}

#[derive(Default)]
struct State {
    received: VecDeque<Bytes>,
    buffered: usize,
    // The peer closed its direction or the connection ended.
    eof: bool,
    reader: Option<Waker>,
    credit: usize,
    // The connection ended: nothing written arrives anymore.
    broken: bool,
    writer: Option<Waker>,
}

#[derive(Default)]
struct Table {
    streams: HashMap<u32, Arc<Mutex<State>>>,
    ended: bool,
}

struct Shared {
    side: Side,
    next_stream: AtomicU32,
    table: Mutex<Table>,
    outgoing: mpsc::UnboundedSender<Out>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn broken_pipe() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "the framed connection ended")
}

fn protocol(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl Shared {
    fn send(&self, stream: u32, kind: FrameKind, payload: Bytes) -> io::Result<()> {
        self.outgoing
            .send(Out::Frame(Frame {
                stream,
                kind,
                payload,
            }))
            .map_err(|_| broken_pipe())
    }

    fn register(self: &Arc<Self>, id: u32, request: Bytes) -> io::Result<Stream> {
        let mut table = lock(&self.table);
        if table.ended {
            return Err(broken_pipe());
        }
        if table.streams.contains_key(&id) {
            return Err(protocol(format!("stream {id} opened twice")));
        }
        let stream = Stream::new(id, request, Arc::clone(self));
        table.streams.insert(id, Arc::clone(&stream.state));

        Ok(stream)
    }

    fn state(&self, id: u32) -> Option<Arc<Mutex<State>>> {
        lock(&self.table).streams.get(&id).cloned()
    }

    fn receive(self: &Arc<Self>, frame: Frame, accepted: &mpsc::Sender<Stream>) -> io::Result<()> {
        let id = frame.stream;
        match frame.kind {
            FrameKind::Open => {
                if id == PIPE || id % 2 == self.side.first_stream() % 2 {
                    return Err(protocol(format!(
                        "the peer opened stream {id} of our parity"
                    )));
                }
                let stream = self.register(id, frame.payload)?;
                // No taker or a full queue refuses the stream: dropping it answers with Close.
                let _ = accepted.try_send(stream);
            }
            FrameKind::Data if frame.payload.is_empty() => {}
            FrameKind::Data => {
                let Some(state) = self.state(id) else {
                    // Our end is gone; granting the bytes back keeps the peer from waiting on them.
                    let granted = (frame.payload.len() as u32).to_be_bytes();
                    let _ = self.send(id, FrameKind::Window, Bytes::copy_from_slice(&granted));
                    return Ok(());
                };
                let mut state = lock(&state);
                if state.buffered + frame.payload.len() > STREAM_WINDOW {
                    return Err(protocol(format!("stream {id} sent past its window")));
                }
                state.buffered += frame.payload.len();
                state.received.push_back(frame.payload);
                if let Some(waker) = state.reader.take() {
                    waker.wake();
                }
            }
            FrameKind::Close => {
                if let Some(state) = self.state(id) {
                    let mut state = lock(&state);
                    state.eof = true;
                    if let Some(waker) = state.reader.take() {
                        waker.wake();
                    }
                }
            }
            FrameKind::Window => {
                if let Some(state) = self.state(id) {
                    let payload = &frame.payload;
                    let granted =
                        u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                    let mut state = lock(&state);
                    state.credit += granted as usize;
                    if let Some(waker) = state.writer.take() {
                        waker.wake();
                    }
                }
            }
        }

        Ok(())
    }

    fn end(&self) {
        let mut table = lock(&self.table);
        table.ended = true;
        for state in table.streams.values() {
            let mut state = lock(state);
            state.eof = true;
            state.broken = true;
            if let Some(waker) = state.reader.take() {
                waker.wake();
            }
            if let Some(waker) = state.writer.take() {
                waker.wake();
            }
        }
    }
}

// Many streams over one byte connection (a container's stdio). Both sides run the same code.
#[derive(Clone)]
pub struct Link {
    shared: Arc<Shared>,
}

impl Link {
    // Returns the link, the browser pipe (stream 0) and the streams the peer opens.
    pub fn start<R, W>(reader: R, writer: W, side: Side) -> (Link, Stream, mpsc::Receiver<Stream>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (outgoing, outgoing_rx) = mpsc::unbounded_channel();
        let (accepted, accepted_rx) = mpsc::channel(ACCEPT_QUEUE);
        let shared = Arc::new(Shared {
            side,
            next_stream: AtomicU32::new(side.first_stream()),
            table: Mutex::default(),
            outgoing,
        });
        let pipe = Stream::new(PIPE, Bytes::new(), Arc::clone(&shared));
        lock(&shared.table)
            .streams
            .insert(PIPE, Arc::clone(&pipe.state));

        tokio::spawn(write_out(writer, outgoing_rx));
        tokio::spawn(demux(reader, Arc::clone(&shared), accepted));

        (Link { shared }, pipe, accepted_rx)
    }

    pub fn open(&self, request: &[u8]) -> io::Result<Stream> {
        if request.len() > MAX_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "an open request of {} bytes is over the {MAX_PAYLOAD} byte cap",
                    request.len()
                ),
            ));
        }
        let id = self.shared.next_stream.fetch_add(2, Ordering::Relaxed);
        let stream = self.shared.register(id, Bytes::copy_from_slice(request))?;
        self.shared
            .send(id, FrameKind::Open, stream.request.clone())?;

        Ok(stream)
    }

    // Resolves once every frame queued before it is written to the connection and flushed.
    pub async fn flush(&self) -> io::Result<()> {
        let (done, flushed) = oneshot::channel();
        self.shared
            .outgoing
            .send(Out::Flushed(done))
            .map_err(|_| broken_pipe())?;

        flushed.await.map_err(|_| broken_pipe())
    }
}

async fn demux<R: AsyncRead + Unpin>(
    reader: R,
    shared: Arc<Shared>,
    accepted: mpsc::Sender<Stream>,
) {
    let mut reader = BufReader::new(reader);
    let mut buffer = BytesMut::new();

    loop {
        let frame = match read_frame(&mut reader, &mut buffer).await {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) => {
                warn!(%error, "framed connection read failed");
                break;
            }
        };
        if let Err(error) = shared.receive(frame, &accepted) {
            warn!(%error, "framed connection broke the protocol");
            break;
        }
    }

    shared.end();
}

// Flushes only when the queue runs dry, so a burst goes out in few writes.
async fn write_out<W: AsyncWrite + Unpin>(writer: W, mut outgoing: mpsc::UnboundedReceiver<Out>) {
    let mut writer = BufWriter::new(writer);
    let mut flushed = Vec::new();

    while let Some(out) = outgoing.recv().await {
        let mut next = Some(out);
        while let Some(out) = next {
            match out {
                Out::Frame(frame) => {
                    if let Err(error) = write_frame(&mut writer, &frame).await {
                        warn!(%error, "framed connection write failed");
                        return;
                    }
                }
                Out::Flushed(done) => flushed.push(done),
            }
            next = outgoing.try_recv().ok();
        }

        if let Err(error) = writer.flush().await {
            warn!(%error, "framed connection write failed");
            return;
        }
        for done in flushed.drain(..) {
            let _ = done.send(());
        }
    }
}

// One direction pair of a Link. Shutting down the write half sends Close; dropping the stream
// closes it on this side, and later data from the peer is discarded.
pub struct Stream {
    id: u32,
    request: Bytes,
    state: Arc<Mutex<State>>,
    shared: Arc<Shared>,
    // Read since the last Window frame.
    consumed: usize,
    closed: bool,
}

impl Stream {
    fn new(id: u32, request: Bytes, shared: Arc<Shared>) -> Stream {
        Stream {
            id,
            request,
            state: Arc::new(Mutex::new(State {
                credit: STREAM_WINDOW,
                ..State::default()
            })),
            shared,
            consumed: 0,
            closed: false,
        }
    }

    pub fn id(&self) -> u32 {
        self.id
    }

    // The peer's Open payload.
    pub fn request(&self) -> &[u8] {
        &self.request
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let mut state = lock(&this.state);
        if state.received.is_empty() {
            if !state.eof {
                state.reader = Some(cx.waker().clone());
                return Poll::Pending;
            }
            return Poll::Ready(Ok(()));
        }

        let mut read = 0;
        while buf.remaining() > 0 {
            let Some(chunk) = state.received.front_mut() else {
                break;
            };
            let n = chunk.len().min(buf.remaining());
            buf.put_slice(&chunk[..n]);
            chunk.advance(n);
            if chunk.is_empty() {
                state.received.pop_front();
            }
            read += n;
        }
        state.buffered -= read;
        drop(state);

        this.consumed += read;
        if this.consumed >= CREDIT_BATCH {
            let granted = (this.consumed as u32).to_be_bytes();
            // A grant fails only when the connection is gone, and with it the peer.
            let _ = this
                .shared
                .send(this.id, FrameKind::Window, Bytes::copy_from_slice(&granted));
            this.consumed = 0;
        }

        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }
        if this.closed {
            return Poll::Ready(Err(broken_pipe()));
        }
        let mut state = lock(&this.state);
        if state.broken {
            return Poll::Ready(Err(broken_pipe()));
        }
        if state.credit == 0 {
            state.writer = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let n = buf.len().min(state.credit).min(MAX_PAYLOAD);
        state.credit -= n;
        drop(state);

        this.shared
            .send(this.id, FrameKind::Data, Bytes::copy_from_slice(&buf[..n]))?;

        Poll::Ready(Ok(n))
    }

    // The writer task flushes whenever its queue runs dry; Link::flush waits for that.
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.closed {
            this.closed = true;
            // Fails only when the connection is gone, which ends the stream for the peer too.
            let _ = this.shared.send(this.id, FrameKind::Close, Bytes::new());
        }

        Poll::Ready(Ok(()))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        lock(&self.shared.table).streams.remove(&self.id);
        if !self.closed {
            let _ = self.shared.send(self.id, FrameKind::Close, Bytes::new());
        }
    }
}
