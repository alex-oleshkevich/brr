use std::path::PathBuf;
use std::time::Duration;

use brr_cdp::{Cdp, CdpEvent};
use brr_core::{Display, DisplayEnv, Engine, Error, InstanceId, ProfileSettings};
use brr_proto::Mux;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::mpsc;

use crate::{Driver, INSTALL_HELP, LaunchSpec};

const DISPLAY_VAR: &str = "DISPLAY";
const SPEC_START_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) fn spec(display: Display, settings: ProfileSettings) -> LaunchSpec {
    LaunchSpec {
        instance_id: InstanceId::new(),
        engine: Engine::Chromium,
        executable: PathBuf::from("chrome"),
        user_data_dir: PathBuf::from("/tmp/profile"),
        log_file: PathBuf::from("/tmp/browser.log"),
        display,
        display_env: DisplayEnv::default(),
        settings,
        start_timeout: SPEC_START_TIMEOUT,
        extensions: Vec::new(),
        proxy_server: None,
        download_dir: PathBuf::from("/tmp/downloads"),
        console_buffer: 1000,
        network_buffer: 1000,
        camoufox: None,
    }
}

#[tokio::test]
async fn visible_needs_a_display() {
    let spec = spec(Display::Visible, ProfileSettings::default());

    let error = Driver::launch(spec).await.err();

    assert_eq!(
        error,
        Some(Error::DependencyMissing {
            name: DISPLAY_VAR.to_owned(),
            help: INSTALL_HELP.to_owned(),
        })
    );
}

const PIPE_BYTES: usize = 64 * 1024;

// The browser side of a pipe CDP connection, for tests that drive the engine without a browser.
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

    pub(crate) async fn answer(&mut self, request: &Value, result: Value) {
        self.send(json!({ "id": request["id"], "result": result }))
            .await;
    }

    pub(crate) async fn event(&mut self, method: &str, params: Value, session_id: Option<&str>) {
        let mut frame = json!({ "method": method, "params": params });
        if let Some(session_id) = session_id {
            frame["sessionId"] = json!(session_id);
        }
        self.send(frame).await;
    }
}

pub(crate) fn fake_cdp() -> (Cdp, mpsc::Receiver<CdpEvent>, Peer) {
    let (ours, theirs) = tokio::io::duplex(PIPE_BYTES);
    let (reader, writer) = tokio::io::split(ours);
    let (mux, incoming) = Mux::start(reader, writer);
    let (cdp, events) = Cdp::start(mux, incoming);
    let (peer_reader, writer) = tokio::io::split(theirs);
    let peer = Peer {
        reader: BufReader::new(peer_reader),
        writer,
    };

    (cdp, events, peer)
}
