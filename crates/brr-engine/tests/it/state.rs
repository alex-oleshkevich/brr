#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_macros)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use brr_core::capture::ResourceType;
use brr_core::{
    Cookie, Display, DisplayEnv, Engine, Error, InstanceId, ProfileSettings, SameSite, TargetId,
    WaitUntil,
};
use brr_engine::page::{NetworkEntry, Page, StorageArea};
use brr_engine::{DownloadState, Driver, DriverEvent, LaunchSpec, Launched, RequestRules};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::mpsc::Receiver;

const CHROME_ENV: &str = "BRR_TEST_CHROME";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(15);
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SLOW_CHUNK_INTERVAL: Duration = Duration::from_millis(20);
// Chromium appends the reason (".Inspector") to what a blocked request reports.
const BLOCKED: &str = "net::ERR_BLOCKED_BY_CLIENT";
const BIG_BYTES: usize = 3 * 1024 * 1024;
const SLOW_DECLARED_BYTES: usize = 100 * 1024 * 1024;
const CHUNK_BYTES: usize = 64 * 1024;
const PAGE_WIDTH: &str = "document.getElementById('i').naturalWidth";
const PIXEL_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

fn chrome() -> Option<PathBuf> {
    let chrome = std::env::var_os(CHROME_ENV).map(PathBuf::from);
    if chrome.is_none() {
        eprintln!("skipped: {CHROME_ENV} is not set");
    }

    chrome
}

fn spec(chrome: PathBuf, dir: &Path) -> LaunchSpec {
    LaunchSpec {
        instance_id: InstanceId::new(),
        engine: Engine::Chromium,
        executable: chrome,
        user_data_dir: dir.join("profile"),
        log_file: dir.join("logs").join("browser.log"),
        display: Display::Headless,
        display_env: DisplayEnv::default(),
        settings: ProfileSettings::default(),
        start_timeout: START_TIMEOUT,
        extensions: Vec::new(),
        proxy_server: None,
        download_dir: dir.join("downloads"),
        console_buffer: 1000,
        network_buffer: 1000,
        camoufox: None,
    }
}

struct Harness {
    driver: Driver,
    target_id: TargetId,
    events: Receiver<DriverEvent>,
    dir: TempDir,
    // 127.0.0.1:<port> of the test server.
    origin: String,
}

impl Harness {
    async fn start(chrome: PathBuf) -> Harness {
        let dir = TempDir::new().unwrap();
        let Launched {
            driver,
            events,
            first_target_id,
            ..
        } = Driver::launch(spec(chrome, dir.path())).await.unwrap();

        Harness {
            driver,
            target_id: first_target_id,
            events,
            dir,
            origin: serve(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.origin)
    }

    async fn open(&self, path: &str) -> Page {
        self.driver
            .navigate(
                &self.target_id,
                &self.url(path),
                WaitUntil::Load,
                NAVIGATE_TIMEOUT,
            )
            .await
            .unwrap();

        self.driver.page(&self.target_id).unwrap()
    }

    async fn wait_for<T>(&mut self, matches: impl Fn(&DriverEvent) -> Option<T>) -> T {
        let found = tokio::time::timeout(EVENT_TIMEOUT, async {
            while let Some(event) = self.events.recv().await {
                if let Some(found) = matches(&event) {
                    return Some(found);
                }
            }
            None
        })
        .await;

        found.unwrap().expect("expected event did not arrive")
    }

    async fn stop(self) {
        self.driver.close().await.unwrap();
    }
}

async fn poll<T>(mut probe: impl FnMut() -> Option<T>) -> T {
    let found = tokio::time::timeout(POLL_TIMEOUT, async {
        loop {
            if let Some(found) = probe() {
                return found;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await;

    found.expect("condition not met in time")
}

fn respond(stream: &mut TcpStream, headers: &str, body: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}

fn handle(mut stream: TcpStream, port: u16) {
    let mut request = [0_u8; 4096];
    let read = stream.read(&mut request).unwrap_or(0);
    let request = String::from_utf8_lossy(&request[..read]);
    let path = request.split_whitespace().nth(1).unwrap_or("/");
    let html = "Content-Type: text/html\r\n";

    match path {
        "/img.png" => respond(
            &mut stream,
            "Content-Type: image/png\r\n",
            &STANDARD.decode(PIXEL_BASE64).unwrap(),
        ),
        "/frame" => respond(&mut stream, html, b"<img id=i src='/img.png'>"),
        "/images" => {
            let page = format!(
                "<img id=i src='/img.png'><iframe src='http://localhost:{port}/frame'></iframe>"
            );
            respond(&mut stream, html, page.as_bytes());
        }
        "/worker" => respond(
            &mut stream,
            html,
            b"<script>new Worker('/worker.js').onmessage = (event) => { document.title = event.data; }</script>",
        ),
        "/worker.js" => respond(
            &mut stream,
            "Content-Type: text/javascript\r\n",
            b"fetch('/data').then(() => postMessage('ok'), () => postMessage('blocked'))",
        ),
        "/console" => respond(
            &mut stream,
            html,
            b"<script>console.log('a'); throw new Error('boom')</script>",
        ),
        "/big" => respond(
            &mut stream,
            "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"big file?.bin\"\r\n",
            &vec![7_u8; BIG_BYTES],
        ),
        "/slow" => {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=slow.bin\r\nContent-Length: {SLOW_DECLARED_BYTES}\r\nConnection: close\r\n\r\n"
            );
            let chunk = [0_u8; CHUNK_BYTES];
            while stream.write_all(&chunk).is_ok() {
                std::thread::sleep(SLOW_CHUNK_INTERVAL);
            }
        }
        _ => respond(&mut stream, html, b"<title>served</title>ok"),
    }
}

// Pages on 127.0.0.1: a secure context, with cookies and storage, unlike a data: URL.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || handle(stream, address.port()));
        }
    });

    address.to_string()
}

fn finished(log: &[NetworkEntry], path: &str) -> Option<NetworkEntry> {
    log.iter()
        .rev()
        .find(|entry| {
            entry.url.ends_with(path) && (entry.status.is_some() || entry.failure.is_some())
        })
        .cloned()
}

fn image_url_count(log: &[NetworkEntry]) -> usize {
    log.iter()
        .filter(|entry| entry.url.ends_with("/img.png"))
        .count()
}

#[tokio::test]
async fn cookies_storage_console() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open("/console").await;

    let messages = poll(|| {
        let messages = page.console_messages();
        (messages.len() >= 2).then_some(messages)
    })
    .await;
    assert!(messages.iter().any(|m| m.level == "log" && m.text == "a"));
    assert!(
        messages
            .iter()
            .any(|m| m.level == "error" && m.text.contains("boom"))
    );

    let cookie = Cookie {
        name: "k".to_owned(),
        value: "v".to_owned(),
        domain: "127.0.0.1".to_owned(),
        path: "/".to_owned(),
        expires: -1.0,
        http_only: false,
        secure: false,
        same_site: SameSite::Lax,
    };
    page.set_cookies(std::slice::from_ref(&cookie))
        .await
        .unwrap();
    let own = page.cookies(&[]).await.unwrap();
    assert!(own.iter().any(|c| c.name == "k" && c.value == "v"));
    let filtered = page.cookies(&[harness.url("/")]).await.unwrap();
    assert!(filtered.iter().any(|c| c.name == "k"));
    let other = page
        .cookies(&["http://example.test/".to_owned()])
        .await
        .unwrap();
    assert!(other.is_empty());

    page.clear_cookies().await.unwrap();
    assert!(page.cookies(&[]).await.unwrap().is_empty());

    let items = BTreeMap::from([("x".to_owned(), "1".to_owned())]);
    page.set_storage(StorageArea::Local, &items).await.unwrap();
    assert_eq!(page.storage(StorageArea::Local).await.unwrap(), items);
    assert!(page.storage(StorageArea::Session).await.unwrap().is_empty());
    page.clear_storage(StorageArea::Local).await.unwrap();
    assert!(page.storage(StorageArea::Local).await.unwrap().is_empty());

    harness
        .driver
        .navigate(
            &harness.target_id,
            "about:blank",
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await
        .unwrap();
    let error = page.storage(StorageArea::Local).await.unwrap_err();
    assert!(matches!(error, Error::InvalidArgument { field, .. } if field == "area"));

    harness.stop().await;
}

#[tokio::test]
async fn block_images_everywhere() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open("/images").await;
    poll(|| (image_url_count(&page.network_log()) >= 2).then_some(())).await;
    let loaded_before = page.network_log().len();

    page.set_request_rules(RequestRules {
        patterns: Vec::new(),
        types: vec![ResourceType::Image],
    })
    .await
    .unwrap();
    let page = harness.open("/images").await;

    let log = poll(|| {
        let log = page.network_log();
        let blocked = log[loaded_before..]
            .iter()
            .filter(|entry| {
                entry
                    .failure
                    .as_deref()
                    .is_some_and(|f| f.starts_with(BLOCKED))
            })
            .count();
        (blocked >= 2).then_some(log)
    })
    .await;
    // The framed image comes from localhost, a different site: its own process and session.
    assert!(
        log[loaded_before..]
            .iter()
            .any(|entry| entry.url.starts_with("http://localhost:") && entry.failure.is_some())
    );
    let width = page.evaluate(PAGE_WIDTH, EVAL_TIMEOUT).await.unwrap();
    assert_eq!(width, json!(0));

    page.set_request_rules(RequestRules::default())
        .await
        .unwrap();
    let page = harness.open("/images").await;
    let width = poll(|| {
        finished(&page.network_log()[loaded_before..], "/img.png").filter(|e| e.status == Some(200))
    })
    .await;
    assert_eq!(width.status, Some(200));
    assert_eq!(
        page.evaluate(PAGE_WIDTH, EVAL_TIMEOUT).await.unwrap(),
        json!(1)
    );

    harness.stop().await;
}

#[tokio::test]
async fn downloads() {
    let Some(chrome) = chrome() else { return };
    let mut harness = Harness::start(chrome).await;
    let page = harness.open("/").await;

    let big = harness.url("/big");
    let tab_id = harness.target_id.clone();
    page.evaluate(&format!("location.href = '{big}'"), EVAL_TIMEOUT)
        .await
        .unwrap();
    let (guid, filename) = harness
        .wait_for(|event| match event {
            DriverEvent::DownloadStarted {
                guid,
                url,
                filename,
                target_id,
            } if *url == big => {
                assert_eq!(target_id.as_ref(), Some(&tab_id));
                Some((guid.clone(), filename.clone()))
            }
            _ => None,
        })
        .await;
    assert_eq!(filename, "big file_.bin");

    let received = harness
        .wait_for(|event| match event {
            DriverEvent::DownloadProgress {
                guid: id,
                received_bytes,
                state: DownloadState::Completed,
                ..
            } if *id == guid => Some(*received_bytes),
            _ => None,
        })
        .await;
    assert_eq!(received, BIG_BYTES as u64);
    let saved = harness.dir.path().join("downloads").join(&guid);
    assert_eq!(std::fs::metadata(saved).unwrap().len(), BIG_BYTES as u64);

    let slow = harness.url("/slow");
    page.evaluate(&format!("location.href = '{slow}'"), EVAL_TIMEOUT)
        .await
        .unwrap();
    let slow_guid = harness
        .wait_for(|event| match event {
            DriverEvent::DownloadStarted { guid, url, .. } if *url == slow => Some(guid.clone()),
            _ => None,
        })
        .await;
    harness.driver.cancel_download(&slow_guid).await.unwrap();
    harness
        .wait_for(|event| match event {
            DriverEvent::DownloadProgress {
                guid,
                state: DownloadState::Canceled,
                ..
            } if *guid == slow_guid => Some(()),
            _ => None,
        })
        .await;

    harness.stop().await;
}

#[tokio::test]
async fn har() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open("/").await;

    page.har_start().unwrap();
    assert!(matches!(
        page.har_start().unwrap_err(),
        Error::InvalidArgument { .. }
    ));
    harness.open("/images").await;
    poll(|| finished(&page.network_log(), "/img.png")).await;

    let har: Value = serde_json::from_slice(&page.har_stop().unwrap()).unwrap();
    assert_eq!(har["log"]["version"], "1.2");
    let entries = har["log"]["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry["request"]["url"] == harness.url("/images")
                && entry["response"]["status"] == 200)
    );
    assert!(matches!(
        page.har_stop().unwrap_err(),
        Error::InvalidArgument { .. }
    ));

    harness.stop().await;
}

// The worker reports through the document title.
async fn worker_verdict(page: &Page) -> Value {
    let verdict = tokio::time::timeout(POLL_TIMEOUT, async {
        loop {
            let title = page.evaluate("document.title", EVAL_TIMEOUT).await.unwrap();
            if title != json!("") {
                return title;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await;

    verdict.expect("the worker did not report")
}

#[tokio::test]
async fn block_requests_in_workers() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open("/").await;

    page.set_request_rules(RequestRules {
        patterns: Vec::new(),
        types: vec![ResourceType::Xhr],
    })
    .await
    .unwrap();
    let page = harness.open("/worker").await;
    assert_eq!(worker_verdict(&page).await, json!("blocked"));

    page.set_request_rules(RequestRules::default())
        .await
        .unwrap();
    let page = harness.open("/worker").await;
    assert_eq!(worker_verdict(&page).await, json!("ok"));

    harness.stop().await;
}
