use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Notify, mpsc, oneshot};
use tracing::{debug, warn};

use crate::{EVENT_QUEUE, Error, FORWARDED, Incoming, MAX_MESSAGE_BYTES};

#[cfg(test)]
mod tests;

const WRITE_QUEUE: usize = 256;
const READ_BUFFER_BYTES: usize = 64 * 1024;
const RETAINED_BUFFER_BYTES: usize = 4 * 1024 * 1024;
const FRAME_DELIMITER: u8 = 0;
const FIRST_CALL_ID: u64 = 1;

enum Reply {
    Caller(oneshot::Sender<Result<Value, Error>>),
    // The answer joins the event stream, in the order the browser sent it, as a FORWARDED event
    // carrying `context` back.
    Stream(Value),
}

#[derive(Clone)]
pub struct Mux {
    shared: Arc<Shared>,
    queue: mpsc::Sender<Vec<u8>>,
}

struct Shared {
    next_id: AtomicU64,
    // None once the connection is gone, so a late call fails instead of waiting forever.
    pending: Mutex<Option<HashMap<u64, Reply>>>,
    closing: Notify,
}

struct PendingGuard<'a> {
    shared: &'a Shared,
    id: u64,
}

enum Frame {
    Message,
    Oversized,
    Eof,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<HashMap<u64, Reply>>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn close(&self) {
        let pending = self.lock().take();
        for reply in pending.into_iter().flatten().map(|(_, reply)| reply) {
            if let Reply::Caller(reply) = reply {
                let _ = reply.send(Err(Error::Closed));
            }
        }
    }
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        if let Some(pending) = self.shared.lock().as_mut() {
            pending.remove(&self.id);
        }
    }
}

impl Mux {
    // Whoever owns the events receiver MUST drain it in its own task. The channel is bounded: a
    // consumer that awaits a call without draining events stalls the reader once EVENT_QUEUE
    // events queue up, and then no response can arrive either.
    pub fn start<R, W>(reader: R, writer: W) -> (Mux, mpsc::Receiver<Incoming>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Mux::start_capped(reader, writer, MAX_MESSAGE_BYTES)
    }

    fn start_capped<R, W>(
        reader: R,
        writer: W,
        max_message_bytes: usize,
    ) -> (Mux, mpsc::Receiver<Incoming>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let shared = Arc::new(Shared {
            next_id: AtomicU64::new(FIRST_CALL_ID),
            pending: Mutex::new(Some(HashMap::new())),
            closing: Notify::new(),
        });
        let (queue, frames) = mpsc::channel(WRITE_QUEUE);
        let (events_tx, events) = mpsc::channel(EVENT_QUEUE);

        tokio::spawn(write_frames(writer, frames, shared.clone()));
        tokio::spawn(read_frames(
            reader,
            events_tx,
            shared.clone(),
            max_message_bytes,
        ));

        (Mux { shared, queue }, events)
    }

    // Ends the connection now, whoever still holds a clone of this Mux: the writer shuts the
    // stream down and every pending and later call fails with Closed.
    pub fn close(&self) {
        self.shared.closing.notify_one();
    }

    pub async fn call(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, Error> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, response) = oneshot::channel();
        self.shared
            .lock()
            .as_mut()
            .ok_or(Error::Closed)?
            .insert(id, Reply::Caller(reply));
        let _guard = PendingGuard {
            shared: &self.shared,
            id,
        };

        let bytes = encode_frame(id, method, &params, session_id)?;
        self.queue.send(bytes).await.map_err(|_| Error::Closed)?;

        response.await.map_err(|_| Error::Closed)?
    }

    // Like `call`, but the answer comes back through the events receiver as a FORWARDED event
    // `{"context", "result"}` or `{"context", "error": {"code", "message"}}`, ordered with the
    // events around it. Nothing is awaited: a closed connection just ends the events.
    pub async fn forward(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
        context: Value,
    ) -> Result<(), Error> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let bytes = encode_frame(id, method, &params, session_id)?;
        self.shared
            .lock()
            .as_mut()
            .ok_or(Error::Closed)?
            .insert(id, Reply::Stream(context));

        if self.queue.send(bytes).await.is_err() {
            if let Some(pending) = self.shared.lock().as_mut() {
                pending.remove(&id);
            }
            return Err(Error::Closed);
        }

        Ok(())
    }
}

// Written straight into one buffer: params can be megabytes (an upload), and building a
// `Value` frame around them first would copy them again.
fn encode_frame(
    id: u64,
    method: &str,
    params: &Value,
    session_id: Option<&str>,
) -> Result<Vec<u8>, Error> {
    let mut bytes = format!("{{\"id\":{id},\"method\":").into_bytes();
    serde_json::to_writer(&mut bytes, method).map_err(io::Error::from)?;
    bytes.extend_from_slice(b",\"params\":");
    serde_json::to_writer(&mut bytes, params).map_err(io::Error::from)?;
    if let Some(session_id) = session_id {
        bytes.extend_from_slice(b",\"sessionId\":");
        serde_json::to_writer(&mut bytes, session_id).map_err(io::Error::from)?;
    }
    bytes.push(b'}');
    bytes.push(FRAME_DELIMITER);

    Ok(bytes)
}

async fn write_frames<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut frames: mpsc::Receiver<Vec<u8>>,
    shared: Arc<Shared>,
) {
    loop {
        let frame = tokio::select! {
            frame = frames.recv() => frame,
            () = shared.closing.notified() => None,
        };
        let Some(frame) = frame else { break };
        let written = async {
            writer.write_all(&frame).await?;
            writer.flush().await
        };
        // A peer that stopped reading blocks the write; close() must still cut the connection.
        let outcome = tokio::select! {
            outcome = written => outcome,
            () = shared.closing.notified() => break,
        };
        if let Err(error) = outcome {
            warn!(%error, "protocol write failed");
            break;
        }
    }
    let _ = writer.shutdown().await;
    shared.close();
}

async fn read_frames<R: AsyncRead + Unpin>(
    reader: R,
    events: mpsc::Sender<Incoming>,
    shared: Arc<Shared>,
    max_message_bytes: usize,
) {
    let mut reader = BufReader::with_capacity(READ_BUFFER_BYTES, reader);
    let mut buffer = Vec::new();

    loop {
        buffer.clear();
        // Frames over READ_BUFFER_BYTES are routine (screencast), so keep their capacity unless a
        // rare outlier grew the buffer past the retained cap.
        if buffer.capacity() > RETAINED_BUFFER_BYTES {
            buffer.shrink_to(READ_BUFFER_BYTES);
        }
        match read_frame(&mut reader, &mut buffer, max_message_bytes).await {
            Ok(Frame::Message) => {}
            Ok(Frame::Oversized) => {
                warn!(
                    max_message_bytes,
                    "protocol frame over the size cap skipped"
                );
                continue;
            }
            Ok(Frame::Eof) => break,
            Err(error) => {
                warn!(%error, "protocol read failed");
                break;
            }
        }

        let Ok(mut frame) = serde_json::from_slice::<Value>(&buffer) else {
            warn!(bytes = buffer.len(), "malformed protocol frame skipped");
            continue;
        };

        if let Some(id) = frame.get("id").and_then(Value::as_u64) {
            if let Some(answer) = resolve(&shared, id, &mut frame) {
                let _ = events.send(answer).await;
            }
        } else if let Some(event) = into_event(&mut frame) {
            // A dropped receiver only means nobody wants events; responses must keep flowing.
            let _ = events.send(event).await;
        } else {
            warn!(
                bytes = buffer.len(),
                "protocol frame without id or method skipped"
            );
        }
    }

    shared.close();
}

// Reads up to the next delimiter without ever buffering more than max_message_bytes: an
// oversized frame is consumed and discarded.
async fn read_frame<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
    max_message_bytes: usize,
) -> io::Result<Frame> {
    let mut oversized = false;

    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            return Ok(Frame::Eof);
        }

        let end = chunk.iter().position(|&byte| byte == FRAME_DELIMITER);
        let body = &chunk[..end.unwrap_or(chunk.len())];
        if buffer.len() + body.len() > max_message_bytes {
            oversized = true;
        }
        if !oversized {
            buffer.extend_from_slice(body);
        }

        let consumed = end.map_or(body.len(), |end| end + 1);
        reader.consume(consumed);

        if end.is_some() {
            return Ok(if oversized {
                Frame::Oversized
            } else {
                Frame::Message
            });
        }
    }
}

fn resolve(shared: &Shared, id: u64, frame: &mut Value) -> Option<Incoming> {
    let reply = shared
        .lock()
        .as_mut()
        .and_then(|pending| pending.remove(&id));
    let Some(reply) = reply else {
        // The common case is a caller that gave up (select, timeout): its guard removed the id.
        debug!(id, "response for a call nobody waits for dropped");
        return None;
    };

    let (reply, context) = match reply {
        Reply::Caller(reply) => (Some(reply), Value::Null),
        Reply::Stream(context) => (None, context),
    };
    let result = match frame.get_mut("error") {
        Some(error) => Err(Error::Remote {
            code: error
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            message: match error.get_mut("message").map(Value::take) {
                Some(Value::String(message)) => message,
                _ => String::new(),
            },
        }),
        None => Ok(frame
            .get_mut("result")
            .map(Value::take)
            .unwrap_or(Value::Null)),
    };
    let Some(reply) = reply else {
        let mut params = json!({ "context": context });
        match result {
            Ok(result) => params["result"] = result,
            Err(Error::Remote { code, message }) => {
                params["error"] = json!({ "code": code, "message": message });
            }
            Err(_) => return None,
        }
        return Some(Incoming {
            method: FORWARDED.to_owned(),
            params,
            session_id: None,
        });
    };
    let _ = reply.send(result);
    None
}

fn into_event(frame: &mut Value) -> Option<Incoming> {
    let Value::String(method) = frame.get_mut("method")?.take() else {
        return None;
    };
    let session_id = match frame.get_mut("sessionId").map(Value::take) {
        Some(Value::String(session_id)) => Some(session_id),
        _ => None,
    };
    let params = frame
        .get_mut("params")
        .map(Value::take)
        .unwrap_or(Value::Null);

    Some(Incoming {
        method,
        params,
        session_id,
    })
}
