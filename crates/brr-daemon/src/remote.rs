use std::path::{Path, PathBuf};

use base64::{Engine, engine::general_purpose::STANDARD};
use brr_core::{Client, ClientId, ClientKind, Error, ErrorBody, Event};
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::{fs::File, io::AsyncWriteExt, net::UnixStream, sync::mpsc, task::JoinHandle};
use tokio_tungstenite::client_async;

use crate::{
    internal,
    wire::{
        ARCHIVE_MIME, CLI_KIND, CLIENT_HEADER, CLIENT_KIND_HEADER, LAGGED, PASSPHRASE_HEADER,
        file_chunks,
    },
};

const BASE_URL: &str = "http://brr";
const WS_BASE_URL: &str = "ws://brr";
const EVENTS_CHANNEL_CAP: usize = 256;
const FRAME_END: &[u8] = b"\n\n";
const LAST_EVENT_ID_HEADER: &str = "Last-Event-ID";
const EVENT_FIELD: &str = "event";
const DATA_FIELD: &str = "data";
pub const DAEMON_NAME: &str = "daemon";
const OUT_MODE: u32 = 0o600;
const DAEMON_HELP: &str = "start it with `brr daemon start`";

#[derive(Clone)]
pub struct Remote {
    http: reqwest::Client,
    socket: PathBuf,
}

pub struct RemoteHold(JoinHandle<()>);

impl Drop for RemoteHold {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl RemoteHold {
    pub fn is_closed(&self) -> bool {
        self.0.is_finished()
    }
}

impl Remote {
    pub fn new(socket: &Path) -> Result<Remote, Error> {
        let http = reqwest::Client::builder()
            .unix_socket(socket)
            .build()
            .map_err(internal)?;

        Ok(Remote {
            http,
            socket: socket.to_path_buf(),
        })
    }

    pub async fn is_up(&self) -> bool {
        UnixStream::connect(&self.socket).await.is_ok()
    }

    pub async fn call(
        &self,
        op: &str,
        input: &Value,
        client_id: Option<&ClientId>,
    ) -> Result<Value, Error> {
        self.post(&format!("/v1/ops/{op}"), input, client_id).await
    }

    // The archive goes up as a streamed body, never read whole into memory.
    pub async fn import_archive(
        &self,
        archive: &Path,
        passphrase: &str,
        name: Option<&str>,
        client_id: &ClientId,
    ) -> Result<Value, Error> {
        let file = File::open(archive).await.map_err(internal)?;
        let chunks = file_chunks(file, ());

        let mut request = self
            .http
            .post(format!("{BASE_URL}/v1/profiles/import/raw"))
            .header(CLIENT_HEADER, client_id.as_str())
            .header(PASSPHRASE_HEADER, STANDARD.encode(passphrase))
            .header("Content-Type", ARCHIVE_MIME)
            .body(reqwest::Body::wrap_stream(chunks));
        if let Some(name) = name {
            request = request.query(&[("name", name)]);
        }
        let response = request.send().await.map_err(request_error)?;
        if !response.status().is_success() {
            return Err(error_from(response).await);
        }

        response.json().await.map_err(internal)
    }

    // Streams the archive to `out` through a temp file, so a failed download leaves nothing behind.
    pub async fn export_archive(
        &self,
        profile: &str,
        passphrase: &str,
        out: &Path,
        client_id: &ClientId,
    ) -> Result<u64, Error> {
        let mut response = self
            .http
            .post(format!("{BASE_URL}/v1/profiles/{profile}/export/raw"))
            .header(CLIENT_HEADER, client_id.as_str())
            .header(PASSPHRASE_HEADER, STANDARD.encode(passphrase))
            .send()
            .await
            .map_err(request_error)?;
        if !response.status().is_success() {
            return Err(error_from(response).await);
        }

        let mut temp_name = out.file_name().unwrap_or_default().to_os_string();
        temp_name.push(format!(".{}.tmp", std::process::id()));
        let temp = out.with_file_name(temp_name);
        let written = write_download(&mut response, &temp).await;
        let renamed = match written {
            Ok(bytes) => tokio::fs::rename(&temp, out)
                .await
                .map(|()| bytes)
                .map_err(internal),
            Err(error) => Err(error),
        };
        if renamed.is_err() {
            let _ = tokio::fs::remove_file(&temp).await;
        }

        renamed
    }

    pub async fn create_client(&self, kind: ClientKind, label: &str) -> Result<Client, Error> {
        let created = self
            .post(
                "/v1/clients",
                &json!({ "kind": kind, "label": label }),
                None,
            )
            .await?;

        serde_json::from_value(created).map_err(internal)
    }

    pub async fn hold(&self, client_id: &ClientId) -> Result<RemoteHold, Error> {
        let stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|_| daemon_missing(DAEMON_HELP))?;
        let url = format!("{WS_BASE_URL}/v1/clients/{client_id}/presence");
        let (mut socket, _) = client_async(url, stream).await.map_err(internal)?;

        // Dropping the socket when the task is aborted closes the connection.
        Ok(RemoteHold(tokio::spawn(async move {
            while let Some(Ok(_)) = socket.next().await {}
        })))
    }

    pub async fn follow_events(
        &self,
        query: &[(&str, String)],
    ) -> Result<mpsc::Receiver<Result<Event, Error>>, Error> {
        let query: Vec<(String, String)> = query
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect();
        let stream = self.open_events(&query, None).await?;
        let (events, received) = mpsc::channel(EVENTS_CHANNEL_CAP);
        let remote = self.clone();

        tokio::spawn(async move { remote.pump_events(stream, &query, events).await });

        Ok(received)
    }

    async fn open_events(
        &self,
        query: &[(String, String)],
        last_seq: Option<i64>,
    ) -> Result<reqwest::Response, Error> {
        let mut request = self
            .http
            .get(format!("{BASE_URL}/v1/events/stream"))
            .query(query);
        if let Some(seq) = last_seq {
            request = request.header(LAST_EVENT_ID_HEADER, seq);
        }

        let response = request.send().await.map_err(request_error)?;
        if !response.status().is_success() {
            return Err(error_from(response).await);
        }

        Ok(response)
    }

    // Reconnects after the server's `lagged` marker, resuming at the last seq delivered.
    async fn pump_events(
        &self,
        mut stream: reqwest::Response,
        query: &[(String, String)],
        events: mpsc::Sender<Result<Event, Error>>,
    ) {
        let mut last_seq = None;

        loop {
            let ended = read_events(&mut stream, &mut last_seq, &events).await;

            let error = match ended {
                StreamEnd::ReceiverGone => return,
                StreamEnd::Lagged => match self.open_events(query, last_seq).await {
                    Ok(reopened) => {
                        stream = reopened;
                        continue;
                    }
                    Err(error) => error,
                },
                StreamEnd::Closed(error) => error,
            };

            let _ = events.send(Err(error)).await;
            return;
        }
    }

    async fn post(
        &self,
        path: &str,
        body: &Value,
        client_id: Option<&ClientId>,
    ) -> Result<Value, Error> {
        let request = self.http.post(format!("{BASE_URL}{path}")).json(body);
        let request = match client_id {
            Some(client_id) => request.header(CLIENT_HEADER, client_id.as_str()),
            None => request.header(CLIENT_KIND_HEADER, CLI_KIND),
        };
        let response = request.send().await.map_err(request_error)?;

        if !response.status().is_success() {
            return Err(error_from(response).await);
        }

        response.json().await.map_err(internal)
    }
}

async fn write_download(response: &mut reqwest::Response, temp: &Path) -> Result<u64, Error> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(OUT_MODE)
        .open(temp)
        .await
        .map_err(internal)?;
    let mut bytes = 0;

    while let Some(chunk) = response.chunk().await.map_err(internal)? {
        bytes += chunk.len() as u64;
        file.write_all(&chunk).await.map_err(internal)?;
    }
    file.flush().await.map_err(internal)?;

    Ok(bytes)
}

pub fn daemon_missing(help: impl Into<String>) -> Error {
    Error::DependencyMissing {
        name: DAEMON_NAME.to_string(),
        help: help.into(),
    }
}

fn request_error(error: reqwest::Error) -> Error {
    if error.is_connect() {
        return daemon_missing(DAEMON_HELP);
    }

    internal(error)
}

async fn error_from(response: reqwest::Response) -> Error {
    match response.json::<ErrorBody>().await {
        Ok(body) => Error::from_body(body),
        Err(e) => internal(e),
    }
}

enum Frame {
    Lagged,
    Event(Event),
    Other,
}

// A field is `name: value`; the space after the colon is optional on the wire.
fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    line.strip_prefix(name)?
        .strip_prefix(':')
        .map(str::trim_start)
}

fn parse_frame(frame: &str) -> Frame {
    if frame
        .lines()
        .any(|line| field(line, EVENT_FIELD) == Some(LAGGED))
    {
        return Frame::Lagged;
    }

    let data: Vec<&str> = frame
        .lines()
        .filter_map(|line| field(line, DATA_FIELD))
        .collect();

    // Frames without a data line (keep-alive comments) are not Events.
    match serde_json::from_str(&data.join("\n")) {
        Ok(event) => Frame::Event(event),
        Err(_) => Frame::Other,
    }
}

enum StreamEnd {
    ReceiverGone,
    Lagged,
    Closed(Error),
}

async fn read_events(
    stream: &mut reqwest::Response,
    last_seq: &mut Option<i64>,
    events: &mpsc::Sender<Result<Event, Error>>,
) -> StreamEnd {
    let mut pending: Vec<u8> = Vec::new();

    loop {
        match stream.chunk().await {
            Ok(Some(chunk)) => pending.extend_from_slice(&chunk),
            Ok(None) => return StreamEnd::Closed(internal("event stream ended; reconnect")),
            Err(e) => return StreamEnd::Closed(internal(e)),
        }

        while let Some(end) = pending
            .windows(FRAME_END.len())
            .position(|w| w == FRAME_END)
        {
            let frame = String::from_utf8_lossy(&pending[..end]).into_owned();
            pending.drain(..end + FRAME_END.len());

            match parse_frame(&frame) {
                Frame::Lagged => return StreamEnd::Lagged,
                Frame::Event(event) => {
                    *last_seq = Some(event.seq);

                    if events.send(Ok(event)).await.is_err() {
                        return StreamEnd::ReceiverGone;
                    }
                }
                Frame::Other => {}
            }
        }
    }
}

#[cfg(test)]
mod tests;
