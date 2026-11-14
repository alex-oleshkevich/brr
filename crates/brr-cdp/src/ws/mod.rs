use std::io::ErrorKind;
use std::time::Duration;

use brr_core::{ConflictReason, Error};
use brr_proto::{MAX_MESSAGE_BYTES, Mux};
use futures::{SinkExt, StreamExt};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::error::ProtocolError;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{WebSocketStream, client_async_with_config};

use crate::{Cdp, CdpEvent};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const BRIDGE_BYTES: usize = 1024 * 1024;
const READ_CHUNK_BYTES: usize = 64 * 1024;
const TCP_TIMEOUT: Duration = Duration::from_secs(5);
const WS_SCHEME: &str = "ws";
const DEFAULT_WS_PORT: u16 = 80;
const INVALID_URL: &str = "invalid URL";
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION_BODY_CAP: usize = 64 * 1024;
const CDP_URL_FIELD: &str = "cdp_url";
const APPROVAL_MODE_MESSAGE: &str = "the browser is in approval mode (/json is disabled); attach by install_name or pass the ws:// URL from DevToolsActivePort";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionInfo {
    web_socket_debugger_url: String,
}

// One attempt only: a retry would show the user a second approval dialog.
pub async fn connect_ws(url: &str) -> Result<(Cdp, mpsc::Receiver<CdpEvent>), Error> {
    let target = Url::parse(url)
        .ok()
        .filter(|target| target.scheme() == WS_SCHEME)
        .ok_or_else(|| unreachable_error(url, &"expected a ws:// URL"))?;
    let host = target
        .host_str()
        .ok_or_else(|| unreachable_error(url, &"missing host"))?;
    let port = target.port_or_known_default().unwrap_or(DEFAULT_WS_PORT);

    let tcp = tokio::time::timeout(TCP_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| unreachable_error(url, &"connection timed out"))?
        .map_err(|error| unreachable_error(url, &error))?;
    tcp.set_nodelay(true)
        .map_err(|error| unreachable_error(url, &error))?;

    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let handshake = client_async_with_config(url, tcp, Some(config));

    let (socket, _) = tokio::time::timeout(CONNECT_TIMEOUT, handshake)
        .await
        .map_err(|_| Error::Conflict {
            reason: ConflictReason::AttachUnanswered,
            detail: None,
        })?
        .map_err(|error| handshake_error(url, &error))?;

    let (ours, theirs) = tokio::io::duplex(BRIDGE_BYTES);
    tokio::spawn(pump(socket, theirs));

    Ok(start_duplex(ours))
}

pub fn start_duplex(stream: DuplexStream) -> (Cdp, mpsc::Receiver<CdpEvent>) {
    let (reader, writer) = tokio::io::split(stream);
    let (mux, incoming) = Mux::start(reader, writer);

    Cdp::start(mux, incoming)
}

pub async fn resolve_http_endpoint(http: &reqwest::Client, url: &str) -> Result<String, Error> {
    if url.starts_with("ws://") || url.starts_with("wss://") {
        return Ok(url.to_owned());
    }

    let endpoint = format!("{}/json/version", url.trim_end_matches('/'));
    let mut response = http
        .get(&endpoint)
        .timeout(VERSION_TIMEOUT)
        .send()
        .await
        .map_err(|error| unreachable_error(url, &error.without_url()))?;
    if response.status() == StatusCode::NOT_FOUND {
        return Err(invalid_url(APPROVAL_MODE_MESSAGE.to_owned()));
    }

    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| unreachable_error(url, &error.without_url()))?
    {
        body.extend_from_slice(&chunk);
        if body.len() > VERSION_BODY_CAP {
            return Err(invalid_url(format!(
                "{} answered /json/version with more than {VERSION_BODY_CAP} bytes",
                redact(url)
            )));
        }
    }

    let info: VersionInfo = serde_json::from_slice(&body).map_err(|error| {
        invalid_url(format!(
            "{} did not answer /json/version like a browser: {error}",
            redact(url)
        ))
    })?;

    Ok(info.web_socket_debugger_url)
}

fn handshake_error(url: &str, error: &WsError) -> Error {
    match error {
        WsError::Http(response)
            if [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN].contains(&response.status()) =>
        {
            declined()
        }
        WsError::Protocol(ProtocolError::HandshakeIncomplete) => declined(),
        WsError::Io(io) if io.kind() == ErrorKind::ConnectionReset => declined(),
        _ => unreachable_error(url, error),
    }
}

fn declined() -> Error {
    Error::Conflict {
        reason: ConflictReason::AttachDeclined,
        detail: None,
    }
}

fn unreachable_error(url: &str, error: &dyn std::fmt::Display) -> Error {
    invalid_url(format!("cannot reach {}: {error}", redact(url)))
}

fn invalid_url(message: String) -> Error {
    Error::InvalidArgument {
        field: CDP_URL_FIELD.to_owned(),
        message,
    }
}

fn redact(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return INVALID_URL.to_owned();
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    // The path of a browser endpoint names the browser (/devtools/browser/<id>).
    parsed.set_path("");
    parsed.set_query(None);
    parsed.set_fragment(None);

    parsed.to_string()
}

async fn pump<S>(mut socket: WebSocketStream<S>, mut bridge: DuplexStream)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut pending = Vec::new();
    let mut scanned = 0;
    let mut chunk = vec![0; READ_CHUNK_BYTES];

    loop {
        tokio::select! {
            message = socket.next() => {
                let bytes: &[u8] = match &message {
                    Some(Ok(Message::Text(text))) => text.as_bytes(),
                    Some(Ok(Message::Binary(bytes))) => bytes,
                    Some(Ok(_)) => continue,
                    _ => break,
                };
                if bridge.write_all(bytes).await.is_err() || bridge.write_all(&[0]).await.is_err() {
                    break;
                }
            }
            read = bridge.read(&mut chunk) => {
                let Ok(read @ 1..) = read else { break };
                pending.extend_from_slice(&chunk[..read]);
                if send_frames(&mut socket, &mut pending, &mut scanned).await.is_err() {
                    break;
                }
            }
        }
    }

    let _ = socket.close(None).await;
}

// A frame may span several reads, so bytes after the last NUL stay in `pending`; `scanned` keeps
// the next search from re-reading them.
async fn send_frames<S>(
    socket: &mut WebSocketStream<S>,
    pending: &mut Vec<u8>,
    scanned: &mut usize,
) -> Result<(), ()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    while let Some(offset) = pending[*scanned..].iter().position(|byte| *byte == 0) {
        let end = *scanned + offset;
        let frame: Vec<u8> = pending.drain(..=end).take(end).collect();
        *scanned = 0;
        let text = String::from_utf8(frame).map_err(|_| ())?;
        socket.send(Message::text(text)).await.map_err(|_| ())?;
    }
    *scanned = pending.len();

    if pending.len() > MAX_MESSAGE_BYTES {
        return Err(());
    }

    Ok(())
}

#[cfg(test)]
mod tests;
