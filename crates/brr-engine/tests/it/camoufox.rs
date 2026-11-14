#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_macros)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use brr_core::capture::{Area, ColorScheme, ImageFormat, ResourceType};

use brr_camoufox::fpgen::model::Model;
use brr_core::{
    Capability, Cookie, Display, DisplayEnv, Engine, Error, Fingerprint, FingerprintOs, InstanceId,
    ProfileSettings, SameSite, Size, TargetId, WaitUntil,
};
use brr_engine::page::{
    ClickOptions, Clip, EmulateOptions, MouseButton, MouseInput, MouseKind, Page,
    ScreenshotOptions, StorageArea, Target,
};
use brr_engine::{
    CamoufoxSpec, DownloadState, Driver, DriverEvent, LaunchSpec, Launched, RequestRules,
};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::mpsc::Receiver;

use crate::common;

use common::{environ, has_xvfb};

const CAMOUFOX_ENV: &str = "BRR_TEST_CAMOUFOX";
const MODEL_ENV: &str = "BRR_TEST_FPGEN_MODEL";
const START_TIMEOUT: Duration = Duration::from_secs(60);
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(30);
const ACTION_TIMEOUT: Duration = Duration::from_secs(10);
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);
const SCREEN: Size = Size {
    width: 1280,
    height: 800,
};
const TITLE_PAGE: &str = "data:text/html,<title>t</title><p>hello</p>";
const TOOLS_PAGE: &str = "data:text/html,<title>start</title><button onclick=\"this.textContent='clicked'\">Go</button><input id=i onkeydown=\"if (event.key == 'Enter') document.title = 'entered'\">";
const POLL_TIMEOUT: Duration = Duration::from_secs(15);
const DELAYED_RESPONSE: Duration = Duration::from_millis(800);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
// Firefox drops a persistent cookie that expires years from now.
const ONE_DAY_S: u64 = 86_400;
const SLOW_DECLARED_BYTES: usize = 100 * 1024 * 1024;
const SLOW_CHUNK_BYTES: usize = 64 * 1024;
const SLOW_CHUNK_INTERVAL: Duration = Duration::from_millis(20);
const BIG_BYTES: usize = 3 * 1024 * 1024;
const VIEWPORT: Size = Size {
    width: 600,
    height: 400,
};
const PAGE_HEIGHT: u32 = 2000;
const SCALE: f64 = 2.0;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const PIXEL_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
const OTHER_PAGE: &str = "data:text/html,<p>other</p>";

pub fn assets() -> Option<(PathBuf, PathBuf, PathBuf)> {
    let Some(camoufox) = std::env::var_os(CAMOUFOX_ENV).map(PathBuf::from) else {
        eprintln!("skipped: {CAMOUFOX_ENV} is not set");
        return None;
    };
    let Some(model) = std::env::var_os(MODEL_ENV).map(PathBuf::from) else {
        eprintln!("skipped: {MODEL_ENV} is not set");
        return None;
    };
    let fonts = camoufox.parent()?.join("fonts");
    if !fonts.is_dir() {
        eprintln!("skipped: {} is not a directory", fonts.display());
        return None;
    }

    Some((camoufox, fonts, model))
}

pub fn spec(
    (camoufox, fonts, model): (PathBuf, PathBuf, PathBuf),
    dir: &Path,
    display: Display,
    screen: Option<Size>,
) -> LaunchSpec {
    LaunchSpec {
        instance_id: InstanceId::new(),
        engine: Engine::Camoufox,
        executable: camoufox,
        user_data_dir: dir.join("profile"),
        log_file: dir.join("logs").join("browser.log"),
        display,
        display_env: DisplayEnv::default(),
        settings: ProfileSettings {
            screen,
            fingerprint: Some(Fingerprint {
                os: Some(FingerprintOs::Linux),
                ..Fingerprint::default()
            }),
            ..ProfileSettings::default()
        },
        start_timeout: START_TIMEOUT,
        extensions: Vec::new(),
        proxy_server: None,
        download_dir: dir.join("downloads"),
        console_buffer: 1000,
        network_buffer: 1000,
        camoufox: Some(CamoufoxSpec {
            fonts_dir: fonts,
            model: Model::open(&model).unwrap(),
            geo: None,
            seed: Some(7),
            addons: Vec::new(),
        }),
    }
}

struct Harness {
    driver: Driver,
    target_id: TargetId,
    events: Receiver<DriverEvent>,
    pid: u32,
    xvfb_pid: Option<u32>,
    _dir: TempDir,
}

impl Harness {
    async fn start(display: Display, screen: Option<Size>) -> Option<Harness> {
        let assets = assets()?;
        let dir = TempDir::new().unwrap();
        let Launched {
            driver,
            events,
            pid,
            first_target_id,
            xvfb_pid,
            ..
        } = Driver::launch(spec(assets, dir.path(), display, screen))
            .await
            .unwrap();

        Some(Harness {
            driver,
            target_id: first_target_id,
            events,
            pid,
            xvfb_pid,
            _dir: dir,
        })
    }

    async fn open(&self, url: &str) -> Page {
        self.driver
            .navigate(&self.target_id, url, WaitUntil::Load, NAVIGATE_TIMEOUT)
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

fn left_click() -> ClickOptions {
    ClickOptions {
        button: MouseButton::Left,
        modifiers: Vec::new(),
        click_count: 1,
    }
}

// `button "Go" [ref=e2]` -> `e2`.
fn ref_of(snapshot: &str, needle: &str) -> String {
    let line = snapshot
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line with {needle} in:\n{snapshot}"));
    let start = line.find("[ref=").unwrap() + "[ref=".len();

    line[start..line[start..].find(']').unwrap() + start].to_owned()
}

async fn eval(page: &Page, expression: &str) -> serde_json::Value {
    page.evaluate(expression, EVAL_TIMEOUT).await.unwrap()
}

#[tokio::test]
async fn launch_and_navigate() {
    let Some(mut harness) = Harness::start(Display::Headless, None).await else {
        return;
    };

    let environ = environ(harness.pid);
    assert!(environ.contains("CAMOU_CONFIG_1="));
    assert!(environ.contains("FONTCONFIG_FILE="));

    harness.open(TITLE_PAGE).await;
    let title = harness
        .wait_for(|event| match event {
            DriverEvent::TabInfo { title, .. } if title == "t" => Some(title.clone()),
            _ => None,
        })
        .await;
    assert_eq!(title, "t");

    harness.stop().await;
}

#[tokio::test]
async fn page_tools() {
    let Some(harness) = Harness::start(Display::Headless, None).await else {
        return;
    };
    let page = harness.open(TOOLS_PAGE).await;

    let snapshot = page.snapshot().await.unwrap();
    let button = Target::Ref(ref_of(&snapshot, "button \"Go\""));
    page.click(&button, &left_click(), ACTION_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(
        eval(&page, "document.querySelector('button').textContent").await,
        json!("clicked")
    );

    let input = Target::Selector("#i".to_owned());
    page.fill(&input, "hello", ACTION_TIMEOUT).await.unwrap();
    assert_eq!(
        eval(&page, "document.getElementById('i').value").await,
        json!("hello")
    );
    page.press_key(Some(&input), "Enter", ACTION_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(eval(&page, "document.title").await, json!("entered"));

    eval(&page, "setTimeout(() => alert('hi'), 0)").await;
    let mut dialog = page.state().dialog();
    tokio::time::timeout(ACTION_TIMEOUT, dialog.wait_for(Option::is_some))
        .await
        .unwrap()
        .unwrap();
    page.handle_dialog(true, None).await.unwrap();
    tokio::time::timeout(ACTION_TIMEOUT, dialog.wait_for(Option::is_none))
        .await
        .unwrap()
        .unwrap();

    let stale = Target::Ref(ref_of(&snapshot, "button \"Go\""));
    let page = harness.open(OTHER_PAGE).await;
    let error = page.resolve(&stale, ACTION_TIMEOUT).await.unwrap_err();
    assert!(matches!(error, Error::StaleRef), "{error:?}");

    harness.stop().await;
}

#[tokio::test]
async fn virtual_display() {
    if !has_xvfb() {
        return;
    }
    let Some(harness) = Harness::start(Display::Virtual, Some(SCREEN)).await else {
        return;
    };
    assert!(harness.xvfb_pid.is_some());
    // The fingerprint fakes screen.width, so the display is checked where the browser runs.
    let environ = environ(harness.pid);
    assert!(environ.lines().any(|line| line.starts_with("DISPLAY=:")));
    assert!(environ.lines().any(|line| line.starts_with("XAUTHORITY=")));
    assert!(
        !environ
            .lines()
            .any(|line| line.starts_with("WAYLAND_DISPLAY="))
    );

    let page = harness.open(TITLE_PAGE).await;
    assert_eq!(eval(&page, "screen.width").await, json!(SCREEN.width));

    harness.stop().await;
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
        "/frame" => respond(&mut stream, html, b"<img src='/img.png'>"),
        "/images" => {
            let page = format!(
                "<script>console.log('hello from the page')</script><img id=i src='/img.png'><iframe src='http://localhost:{port}/frame'></iframe>"
            );
            respond(&mut stream, html, page.as_bytes());
        }
        "/tall" => {
            let page = format!(
                "<body style='margin:0;height:{PAGE_HEIGHT}px'><div id=box style='position:absolute;top:100px;width:200px;height:100px;background:red'></div></body>"
            );
            respond(&mut stream, html, page.as_bytes());
        }
        "/inner" => respond(
            &mut stream,
            html,
            b"<button onclick=\"parent.document.title = 'inner clicked'\">Inside</button>",
        ),
        "/iframe" => respond(
            &mut stream,
            html,
            b"<body style='height:3000px'><iframe src='/inner' style='border:4px solid;padding:3px'></iframe></body>",
        ),
        "/xhr" => respond(
            &mut stream,
            html,
            b"<script>setTimeout(() => fetch('/delayed').then(() => { document.title = 'done'; }), 100)</script>",
        ),
        "/delayed" => {
            std::thread::sleep(DELAYED_RESPONSE);
            respond(&mut stream, "Content-Type: text/plain\r\n", b"late");
        }
        "/slow" => {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=slow.bin\r\nContent-Length: {SLOW_DECLARED_BYTES}\r\nConnection: close\r\n\r\n"
            );
            let chunk = [0_u8; SLOW_CHUNK_BYTES];
            while stream.write_all(&chunk).is_ok() {
                std::thread::sleep(SLOW_CHUNK_INTERVAL);
            }
        }
        "/big" => respond(
            &mut stream,
            "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"big.bin\"\r\n",
            &vec![7_u8; BIG_BYTES],
        ),
        _ => respond(&mut stream, html, b"<title>served</title>ok"),
    }
}

// Pages on 127.0.0.1: a web origin, with cookies and storage, unlike a data: URL.
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

fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(bytes[..8], PNG_SIGNATURE);
    let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());

    (word(16), word(20))
}

fn shot(clip: Clip, format: ImageFormat) -> ScreenshotOptions {
    ScreenshotOptions {
        clip,
        format,
        quality: Some(60),
        transparent: false,
    }
}

#[tokio::test]
async fn capture_state() {
    let Some(harness) = Harness::start(Display::Headless, None).await else {
        return;
    };
    let origin = serve();
    let page = harness.open(&format!("http://{origin}/tall")).await;

    page.emulate(&EmulateOptions {
        viewport: Some(VIEWPORT),
        color_scheme: Some(ColorScheme::Dark),
        ..EmulateOptions::default()
    })
    .await
    .unwrap();
    assert_eq!(eval(&page, "innerWidth").await, json!(VIEWPORT.width));
    assert_eq!(
        eval(&page, "matchMedia('(prefers-color-scheme: dark)').matches").await,
        json!(true)
    );

    let viewport = page
        .screenshot(&shot(Clip::Viewport, ImageFormat::Png))
        .await
        .unwrap();
    assert_eq!(png_size(&viewport), (VIEWPORT.width, VIEWPORT.height));
    let full = page
        .screenshot(&shot(Clip::FullPage, ImageFormat::Png))
        .await
        .unwrap();
    assert_eq!(png_size(&full), (VIEWPORT.width, PAGE_HEIGHT));
    let element = page
        .screenshot(&shot(
            Clip::Element(Target::Selector("#box".to_owned())),
            ImageFormat::Png,
        ))
        .await
        .unwrap();
    assert_eq!(png_size(&element), (200, 100));
    let area = page
        .screenshot(&shot(
            Clip::Area(Area {
                x: 10.0,
                y: 20.0,
                width: 50.0,
                height: 30.0,
            }),
            ImageFormat::Png,
        ))
        .await
        .unwrap();
    assert_eq!(png_size(&area), (50, 30));

    let jpeg = page
        .screenshot(&shot(Clip::Viewport, ImageFormat::Jpeg))
        .await
        .unwrap();
    assert_eq!(jpeg[..3], [0xFF, 0xD8, 0xFF]);
    let webp = page
        .screenshot(&shot(Clip::Viewport, ImageFormat::Webp))
        .await
        .unwrap();
    assert_eq!((&webp[..4], &webp[8..12]), (&b"RIFF"[..], &b"WEBP"[..]));

    let error = page.pdf(&Default::default()).await.unwrap_err();
    assert!(matches!(
        error,
        Error::EngineUnsupported {
            capability: Capability::Pdf
        }
    ));
    let mut transparent = shot(Clip::Viewport, ImageFormat::Png);
    transparent.transparent = true;
    let error = page.screenshot(&transparent).await.unwrap_err();
    assert!(matches!(error, Error::InvalidArgument { field, .. } if field == "transparent"));

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
    assert_eq!(own, [cookie]);
    assert!(
        page.cookies(&["http://example.test/".to_owned()])
            .await
            .unwrap()
            .is_empty()
    );
    page.clear_cookies().await.unwrap();
    assert!(page.cookies(&[]).await.unwrap().is_empty());

    let items = BTreeMap::from([("x".to_owned(), "1".to_owned())]);
    page.set_storage(StorageArea::Local, &items).await.unwrap();
    assert_eq!(page.storage(StorageArea::Local).await.unwrap(), items);

    page.har_start().unwrap();
    let page = harness.open(&format!("http://{origin}/tall")).await;
    let har: serde_json::Value = serde_json::from_slice(&page.har_stop().unwrap()).unwrap();
    assert_eq!(har["log"]["entries"][0]["response"]["status"], 200);

    harness.stop().await;
}

fn image_entries(page: &Page) -> Vec<brr_engine::page::NetworkEntry> {
    page.network_log()
        .into_iter()
        .filter(|entry| entry.url.ends_with("/img.png"))
        .collect()
}

#[tokio::test]
async fn interception() {
    let Some(harness) = Harness::start(Display::Headless, None).await else {
        return;
    };
    let origin = serve();
    let page = harness.open(&format!("http://{origin}/images")).await;
    poll(|| (image_entries(&page).len() >= 2).then_some(())).await;
    let before = page.network_log().len();

    page.set_request_rules(RequestRules {
        patterns: Vec::new(),
        types: vec![ResourceType::Image],
    })
    .await
    .unwrap();
    let page = harness.open(&format!("http://{origin}/images")).await;

    let blocked = poll(|| {
        let blocked: Vec<_> = page.network_log()[before..]
            .iter()
            .filter(|entry| entry.url.ends_with("/img.png") && entry.failure.is_some())
            .cloned()
            .collect();
        (blocked.len() >= 2).then_some(blocked)
    })
    .await;
    assert!(
        blocked
            .iter()
            .any(|entry| entry.url.starts_with("http://localhost:"))
    );
    assert!(
        page.console_messages()
            .iter()
            .any(|message| message.text == "hello from the page")
    );

    harness.stop().await;
}

#[tokio::test]
async fn contexts_downloads() {
    let Some(mut harness) = Harness::start(Display::Headless, None).await else {
        return;
    };
    let origin = serve();
    let url = format!("http://{origin}/");
    let first = harness.driver.create_context().await.unwrap();
    let second = harness.driver.create_context().await.unwrap();
    let in_first = harness.driver.open_tab_in(&first, &url).await.unwrap();
    let in_second = harness.driver.open_tab_in(&second, &url).await.unwrap();
    let page_one = poll(|| harness.driver.page(&in_first).ok()).await;
    let page_two = poll(|| harness.driver.page(&in_second).ok()).await;

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
    page_one
        .set_cookies(std::slice::from_ref(&cookie))
        .await
        .unwrap();
    assert_eq!(
        page_one.cookies(std::slice::from_ref(&url)).await.unwrap(),
        [cookie]
    );
    assert!(
        page_two
            .cookies(std::slice::from_ref(&url))
            .await
            .unwrap()
            .is_empty()
    );
    harness.driver.dispose_context(&second).await.unwrap();

    let big = format!("{url}big");
    let page = harness.open(&url).await;
    page.evaluate(&format!("location.href = '{big}'"), EVAL_TIMEOUT)
        .await
        .unwrap();
    let guid = harness
        .wait_for(|event| match event {
            DriverEvent::DownloadStarted { guid, url, .. } if *url == big => Some(guid.clone()),
            _ => None,
        })
        .await;
    let declared = harness
        .wait_for(|event| match event {
            DriverEvent::DownloadProgress {
                guid: id,
                total_bytes,
                state: DownloadState::InProgress,
                ..
            } if *id == guid => Some(*total_bytes),
            _ => None,
        })
        .await;
    assert_eq!(declared, Some(BIG_BYTES as u64));
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
    let saved = harness._dir.path().join("downloads").join(&guid);
    assert_eq!(std::fs::metadata(saved).unwrap().len(), BIG_BYTES as u64);

    harness.stop().await;
}

#[tokio::test]
async fn cookies_reach_the_profile_files() {
    let Some(harness) = Harness::start(Display::Headless, None).await else {
        return;
    };
    let origin = serve();
    let page = harness.open(&format!("http://{origin}/")).await;
    let cookie = Cookie {
        name: "kept".to_owned(),
        value: "v".to_owned(),
        domain: "127.0.0.1".to_owned(),
        path: "/".to_owned(),
        expires: (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + ONE_DAY_S) as f64,
        http_only: true,
        secure: false,
        same_site: SameSite::Strict,
    };
    page.set_cookies(std::slice::from_ref(&cookie))
        .await
        .unwrap();
    let profile = harness._dir.path().join("profile");
    harness.driver.close().await.unwrap();

    let read = brr_engine::read_cookies_offline(&profile).unwrap();
    assert!(read.contains(&cookie), "{read:?}");
}

#[tokio::test]
async fn tab_operations() {
    let Some(harness) = Harness::start(Display::Headless, None).await else {
        return;
    };
    let origin = serve();
    let url = |path: &str| format!("http://{origin}{path}");

    let target_id = harness.driver.open_tab(&url("/iframe")).await.unwrap();
    let page = poll(|| harness.driver.page(&target_id).ok()).await;
    harness.driver.activate_tab(&target_id).await.unwrap();
    // open_tab does not wait for the load, so the navigation below does.
    harness
        .driver
        .navigate(
            &target_id,
            &url("/iframe"),
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await
        .unwrap();

    let snapshot = page.snapshot().await.unwrap();
    let inside = Target::Ref(ref_of(&snapshot, "button \"Inside\""));
    page.click(&inside, &left_click(), ACTION_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(eval(&page, "document.title").await, json!("inner clicked"));

    page.scroll_by(0.0, 300.0).await.unwrap();
    // Firefox scrolls smoothly, so the offset arrives after the wheel call returns.
    tokio::time::timeout(POLL_TIMEOUT, async {
        while eval(&page, "scrollY").await.as_f64().unwrap() <= 0.0 {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .expect("the page did not scroll");
    page.mouse(&MouseInput {
        kind: MouseKind::Wheel,
        x: 100.0,
        y: 100.0,
        button: MouseButton::Left,
        click_count: 0,
        modifiers: 0,
        delta_x: 0.0,
        delta_y: 100.0,
    })
    .await
    .unwrap();

    harness
        .driver
        .navigate(
            &target_id,
            &url("/second"),
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await
        .unwrap();
    assert!(
        page.go_back(WaitUntil::Load, NAVIGATE_TIMEOUT)
            .await
            .unwrap()
    );
    assert!(
        eval(&page, "location.href")
            .await
            .as_str()
            .unwrap()
            .ends_with("/iframe")
    );
    assert!(
        page.go_forward(WaitUntil::Load, NAVIGATE_TIMEOUT)
            .await
            .unwrap()
    );
    page.reload(WaitUntil::Load, NAVIGATE_TIMEOUT)
        .await
        .unwrap();
    assert!(
        eval(&page, "location.href")
            .await
            .as_str()
            .unwrap()
            .ends_with("/second")
    );

    harness.driver.close_tab(&target_id).await.unwrap();
    poll(|| harness.driver.page(&target_id).is_err().then_some(())).await;

    harness.stop().await;
}

#[tokio::test]
async fn a_running_download_reports_its_size_so_it_can_be_cancelled() {
    let Some(mut harness) = Harness::start(Display::Headless, None).await else {
        return;
    };
    let origin = serve();
    let page = harness.open(&format!("http://{origin}/")).await;
    let slow = format!("http://{origin}/slow");
    page.evaluate(&format!("location.href = '{slow}'"), EVAL_TIMEOUT)
        .await
        .unwrap();
    let guid = harness
        .wait_for(|event| match event {
            DriverEvent::DownloadStarted { guid, url, .. } if *url == slow => Some(guid.clone()),
            _ => None,
        })
        .await;

    let received = harness
        .wait_for(|event| match event {
            DriverEvent::DownloadProgress {
                guid: id,
                received_bytes,
                total_bytes,
                state: DownloadState::InProgress,
            } if *id == guid && *received_bytes > 0 => {
                assert_eq!(*total_bytes, Some(SLOW_DECLARED_BYTES as u64));
                Some(*received_bytes)
            }
            _ => None,
        })
        .await;
    assert!(received < SLOW_DECLARED_BYTES as u64);

    harness.driver.cancel_download(&guid).await.unwrap();
    harness
        .wait_for(|event| match event {
            DriverEvent::DownloadProgress {
                guid: id,
                state: DownloadState::Canceled,
                ..
            } if *id == guid => Some(()),
            _ => None,
        })
        .await;

    harness.stop().await;
}

#[tokio::test]
async fn attached_precedes_loaded() {
    let Some(mut harness) = Harness::start(Display::Headless, None).await else {
        return;
    };
    let attached = std::cell::Cell::new(false);
    let first = harness.target_id.clone();

    let loaded_after_attached = harness
        .wait_for(|event| match event {
            DriverEvent::TabAttached { target_id } if *target_id == first => {
                attached.set(true);
                None
            }
            DriverEvent::TabLoaded { target_id } if *target_id == first => Some(attached.get()),
            _ => None,
        })
        .await;

    assert!(loaded_after_attached);
    harness.stop().await;
}

// A headed window's first tab has no document until the launch loads one; a second tab would be
// a ghost that never loads.
#[tokio::test]
async fn headed_first_tab_loads() {
    if !has_xvfb() {
        return;
    }
    let Some(mut harness) = Harness::start(Display::Virtual, Some(SCREEN)).await else {
        return;
    };
    let first = harness.target_id.clone();

    harness
        .wait_for(|event| match event {
            DriverEvent::TabLoaded { target_id } if *target_id == first => Some(()),
            _ => None,
        })
        .await;
    let ready = eval(&harness.driver.page(&first).unwrap(), "document.readyState").await;
    let others = std::iter::from_fn(|| harness.events.try_recv().ok())
        .filter(|event| matches!(event, DriverEvent::TabCreated { target_id, .. } if *target_id != first))
        .count();

    assert_eq!(ready, "complete");
    assert_eq!(others, 0);
    harness.stop().await;
}

#[tokio::test]
async fn networkidle_waits_for_the_page_requests() {
    let Some(harness) = Harness::start(Display::Headless, None).await else {
        return;
    };
    let origin = serve();

    harness
        .driver
        .navigate(
            &harness.target_id,
            &format!("http://{origin}/xhr"),
            WaitUntil::Networkidle,
            NAVIGATE_TIMEOUT,
        )
        .await
        .unwrap();

    let page = harness.driver.page(&harness.target_id).unwrap();
    assert_eq!(eval(&page, "document.title").await, json!("done"));

    harness.stop().await;
}

#[tokio::test]
async fn a_device_scale_is_refused_not_dropped() {
    let Some(harness) = Harness::start(Display::Headless, None).await else {
        return;
    };
    let page = harness.open(TITLE_PAGE).await;

    let refused = page
        .emulate(&EmulateOptions {
            scale: Some(SCALE),
            ..EmulateOptions::default()
        })
        .await;

    assert!(
        matches!(&refused, Err(Error::InvalidArgument { field, .. }) if field == "scale"),
        "{refused:?}"
    );

    harness.stop().await;
}
