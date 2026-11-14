#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use brr_core::capture::{Area, ColorScheme, ImageFormat, PdfOptions};
use brr_core::{
    Display, DisplayEnv, Engine as BrowserEngine, Error, Geolocation, InstanceId, ProfileSettings,
    Size, TargetId, WaitUntil,
};
use brr_engine::page::{Clip, EmulateOptions, Page, ScreenshotOptions, Target};
use brr_engine::{Driver, DriverEvent, LaunchSpec, Launched};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::mpsc::Receiver;

const CHROME_ENV: &str = "BRR_TEST_CHROME";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(20);
const EVAL_TIMEOUT: Duration = Duration::from_secs(60);
const EVENT_TIMEOUT: Duration = Duration::from_secs(15);
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const TALL_PAGE: &str = "data:text/html,<body style=margin:0><div id=box style='position:absolute;top:200px;left:20px;width:100px;height:50px;background:red'></div><div style='height:3000px'></div></body>";
const FOLD_PAGE: &str = "data:text/html,<body style=margin:0><div id=far style='position:absolute;top:2000px;left:30px;width:100px;height:50px;background:red'></div><div style='height:3000px'></div></body>";
const SCALE_PAGE: &str =
    "data:text/html,<body style=margin:0><div style='height:5000px'></div></body>";
const SCALE: f64 = 4.0;
const PDF_PAGE: &str = "data:text/html,<h1>Hello</h1><p>printed</p>";
const BIG_PDF_BYTES: usize = 20 * 1024 * 1024;
const GEOLOCATION_SCRIPT: &str = "new Promise((resolve) => navigator.geolocation.getCurrentPosition((p) => resolve([p.coords.latitude, p.coords.longitude]), (e) => resolve(String(e.message))))";

// A page of random, incompressible canvases: each is 16 MB of pixels.
fn canvases(count: usize) -> String {
    format!(
        "data:text/html,<body><script>for (let n = 0; n < {count}; n++) {{ const canvas = document.createElement('canvas'); canvas.width = 2000; canvas.height = 2000; const context = canvas.getContext('2d'); const image = context.createImageData(2000, 2000); for (let offset = 0; offset < image.data.length; offset += 65536) crypto.getRandomValues(image.data.subarray(offset, offset + 65536)); context.putImageData(image, 0, 0); document.body.append(canvas); }}</script></body>"
    )
}

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
        engine: BrowserEngine::Chromium,
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
    _dir: TempDir,
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
            _dir: dir,
        }
    }

    async fn open(&self, url: &str) -> Page {
        self.driver
            .navigate(&self.target_id, url, WaitUntil::Load, NAVIGATE_TIMEOUT)
            .await
            .unwrap();

        self.driver.page(&self.target_id).unwrap()
    }

    // Every target's event, in whatever order they arrive.
    async fn wait_for_all(
        &mut self,
        targets: &[TargetId],
        event: impl Fn(&TargetId) -> DriverEvent,
    ) {
        let mut missing: Vec<DriverEvent> = targets.iter().map(event).collect();
        let found = tokio::time::timeout(EVENT_TIMEOUT, async {
            while !missing.is_empty() {
                let Some(next) = self.events.recv().await else {
                    return false;
                };
                missing.retain(|wanted| *wanted != next);
            }
            true
        })
        .await;

        assert_eq!(found, Ok(true), "expected events did not arrive");
    }

    async fn wait_for(&mut self, matches: impl Fn(&DriverEvent) -> bool) {
        let found = tokio::time::timeout(EVENT_TIMEOUT, async {
            while let Some(event) = self.events.recv().await {
                if matches(&event) {
                    return true;
                }
            }
            false
        })
        .await;

        assert_eq!(found, Ok(true), "expected event did not arrive");
    }
}

// A page on 127.0.0.1: a secure context, with cookies, unlike a data: URL.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let body = "<title>served</title>ok";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });

    url
}

fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(bytes[..8], PNG_SIGNATURE);
    let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());

    (word(16), word(20))
}

fn jpeg_size(bytes: &[u8]) -> (u32, u32) {
    assert_eq!(bytes[..2], [0xFF, 0xD8]);
    let mut at = 2;
    while at + 9 < bytes.len() {
        assert_eq!(bytes[at], 0xFF, "marker expected at {at}");
        let marker = bytes[at + 1];
        if (0xC0..=0xC2).contains(&marker) {
            let word =
                |offset: usize| u32::from(u16::from_be_bytes([bytes[offset], bytes[offset + 1]]));

            return (word(at + 7), word(at + 5));
        }
        at += 2 + usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
    }

    panic!("no frame header in the jpeg");
}

fn options(clip: Clip, format: ImageFormat) -> ScreenshotOptions {
    ScreenshotOptions {
        clip,
        format,
        quality: (format == ImageFormat::Jpeg).then_some(50),
        transparent: false,
    }
}

// One pixel as [r, g, b, a], decoded by the browser itself.
async fn pixel_at(page: &Page, png: &[u8], x: u32, y: u32) -> Vec<u64> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    let script = format!(
        "(async () => {{ const blob = await (await fetch('data:image/png;base64,{encoded}')).blob(); const bitmap = await createImageBitmap(blob); const canvas = new OffscreenCanvas(bitmap.width, bitmap.height); const context = canvas.getContext('2d'); context.drawImage(bitmap, 0, 0); return Array.from(context.getImageData({x}, {y}, 1, 1).data); }})()"
    );
    let pixel = page.evaluate(&script, EVAL_TIMEOUT).await.unwrap();

    pixel
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect()
}

#[tokio::test]
async fn screenshots() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open(TALL_PAGE).await;
    page.emulate(&EmulateOptions {
        viewport: Some(Size {
            width: 800,
            height: 600,
        }),
        ..EmulateOptions::default()
    })
    .await
    .unwrap();

    let viewport = page
        .screenshot(&options(Clip::Viewport, ImageFormat::Png))
        .await
        .unwrap();
    assert_eq!(png_size(&viewport), (800, 600));

    let full = page
        .screenshot(&options(Clip::FullPage, ImageFormat::Png))
        .await
        .unwrap();
    assert_eq!(png_size(&full), (800, 3000));

    let element = Clip::Element(Target::Selector("#box".to_owned()));
    let boxed = page
        .screenshot(&options(element, ImageFormat::Png))
        .await
        .unwrap();
    assert_eq!(png_size(&boxed), (100, 50));

    let area = Clip::Area(Area {
        x: 10.0,
        y: 10.0,
        width: 40.0,
        height: 30.0,
    });
    let cropped = page
        .screenshot(&options(area.clone(), ImageFormat::Png))
        .await
        .unwrap();
    assert_eq!(png_size(&cropped), (40, 30));

    let viewport_jpeg = page
        .screenshot(&options(Clip::Viewport, ImageFormat::Jpeg))
        .await
        .unwrap();
    assert_eq!(jpeg_size(&viewport_jpeg), (800, 600));
    let element_jpeg = page
        .screenshot(&options(
            Clip::Element(Target::Selector("#box".to_owned())),
            ImageFormat::Jpeg,
        ))
        .await
        .unwrap();
    assert_eq!(jpeg_size(&element_jpeg), (100, 50));

    let jpeg = page
        .screenshot(&options(area, ImageFormat::Jpeg))
        .await
        .unwrap();
    assert_eq!(jpeg_size(&jpeg), (40, 30));
    let full_jpeg = page
        .screenshot(&options(Clip::FullPage, ImageFormat::Jpeg))
        .await
        .unwrap();
    assert_eq!(jpeg_size(&full_jpeg), (800, 3000));

    let mut transparent = options(Clip::Viewport, ImageFormat::Png);
    transparent.transparent = true;
    let clear = page.screenshot(&transparent).await.unwrap();
    assert_eq!(pixel_at(&page, &clear, 500, 500).await[3], 0);
    let opaque = page
        .screenshot(&options(Clip::Viewport, ImageFormat::Png))
        .await
        .unwrap();
    assert_eq!(pixel_at(&page, &opaque, 500, 500).await[3], 255);

    harness.driver.close().await.unwrap();
}

#[tokio::test]
async fn pdf_and_mhtml() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open(PDF_PAGE).await;

    let a4 = PdfOptions {
        paper: Some("A4".to_owned()),
        pages: Some("1".to_owned()),
        ..PdfOptions::default()
    };
    assert!(page.pdf(&a4).await.unwrap().starts_with(b"%PDF-"));

    let landscape = PdfOptions {
        paper: Some("Letter".to_owned()),
        landscape: Some(true),
        ..PdfOptions::default()
    };
    assert!(page.pdf(&landscape).await.unwrap().starts_with(b"%PDF-"));

    let mhtml = String::from_utf8(page.mhtml().await.unwrap()).unwrap();
    assert!(mhtml.contains("MIME-Version"));

    let page = harness.open(&canvases(5)).await;
    let big = page.pdf(&PdfOptions::default()).await.unwrap();
    assert!(big.len() > BIG_PDF_BYTES, "{} bytes", big.len());

    harness.driver.close().await.unwrap();
}

#[tokio::test]
async fn emulation() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open(&serve()).await;

    page.emulate(&EmulateOptions {
        color_scheme: Some(ColorScheme::Dark),
        timezone: Some("Asia/Tokyo".to_owned()),
        locale: Some("de-DE".to_owned()),
        geolocation: Some(Geolocation {
            latitude: 35.0,
            longitude: 139.0,
            accuracy: None,
        }),
        viewport: Some(Size {
            width: 500,
            height: 400,
        }),
        ..EmulateOptions::default()
    })
    .await
    .unwrap();

    let eval = |expression: &'static str| page.evaluate(expression, EVAL_TIMEOUT);
    assert_eq!(
        eval("matchMedia('(prefers-color-scheme: dark)').matches")
            .await
            .unwrap(),
        json!(true)
    );
    assert_eq!(
        eval("Intl.DateTimeFormat().resolvedOptions().timeZone")
            .await
            .unwrap(),
        json!("Asia/Tokyo")
    );
    assert_eq!(eval("navigator.language").await.unwrap(), json!("de-DE"));
    assert_eq!(eval("innerWidth").await.unwrap(), json!(500));
    assert_eq!(eval(GEOLOCATION_SCRIPT).await.unwrap(), json!([35, 139]));

    // A later call overrides only what it names; the other media feature stays.
    page.emulate(&EmulateOptions {
        reduced_motion: Some(true),
        ..EmulateOptions::default()
    })
    .await
    .unwrap();
    assert_eq!(
        eval("matchMedia('(prefers-color-scheme: dark)').matches")
            .await
            .unwrap(),
        json!(true)
    );
    assert_eq!(
        eval("matchMedia('(prefers-reduced-motion: reduce)').matches")
            .await
            .unwrap(),
        json!(true)
    );

    let invalid = page
        .emulate(&EmulateOptions {
            timezone: Some("Mars/X".to_owned()),
            ..EmulateOptions::default()
        })
        .await;
    assert!(
        matches!(&invalid, Err(Error::InvalidArgument { field, .. }) if field == "timezone"),
        "{invalid:?}"
    );
    // A later call changes only what it names: the device's user agent survives a locale, and
    // the locale survives a user agent.
    page.emulate(&EmulateOptions {
        device: Some("iPhone 15".to_owned()),
        ..EmulateOptions::default()
    })
    .await
    .unwrap();
    page.emulate(&EmulateOptions {
        locale: Some("fr-FR".to_owned()),
        timezone: Some("Europe/Paris".to_owned()),
        ..EmulateOptions::default()
    })
    .await
    .unwrap();
    assert_eq!(
        eval("navigator.userAgent.includes('iPhone')")
            .await
            .unwrap(),
        json!(true)
    );
    assert_eq!(eval("navigator.language").await.unwrap(), json!("fr-FR"));
    assert_eq!(
        eval("Intl.DateTimeFormat().resolvedOptions().timeZone")
            .await
            .unwrap(),
        json!("Europe/Paris")
    );
    page.emulate(&EmulateOptions {
        user_agent: Some("brr-test-agent".to_owned()),
        ..EmulateOptions::default()
    })
    .await
    .unwrap();
    assert_eq!(
        eval("navigator.userAgent").await.unwrap(),
        json!("brr-test-agent")
    );
    assert_eq!(eval("navigator.language").await.unwrap(), json!("fr-FR"));

    let unknown = page
        .emulate(&EmulateOptions {
            device: Some("Nope".to_owned()),
            ..EmulateOptions::default()
        })
        .await;
    assert!(matches!(unknown, Err(Error::InvalidArgument { field, .. }) if field == "device"));

    harness.driver.close().await.unwrap();
}

#[tokio::test]
async fn contexts_isolated() {
    let Some(chrome) = chrome() else { return };
    let mut harness = Harness::start(chrome).await;
    let url = serve();

    let first = harness.driver.create_context().await.unwrap();
    let second = harness.driver.create_context().await.unwrap();
    let tab_one = harness.driver.open_tab_in(&first, &url).await.unwrap();
    let tab_two = harness.driver.open_tab_in(&second, &url).await.unwrap();
    harness
        .wait_for_all(&[tab_one.clone(), tab_two.clone()], |tab| {
            DriverEvent::TabAttached {
                target_id: tab.clone(),
            }
        })
        .await;
    for tab in [&tab_one, &tab_two] {
        harness
            .driver
            .navigate(tab, &url, WaitUntil::Load, NAVIGATE_TIMEOUT)
            .await
            .unwrap();
    }

    let page_one = harness.driver.page(&tab_one).unwrap();
    let page_two = harness.driver.page(&tab_two).unwrap();
    let set = page_one
        .evaluate("document.cookie = 'a=1'; document.cookie", EVAL_TIMEOUT)
        .await
        .unwrap();
    assert_eq!(set, json!("a=1"));
    assert_eq!(
        page_two
            .evaluate("document.cookie", EVAL_TIMEOUT)
            .await
            .unwrap(),
        json!("")
    );

    harness.driver.dispose_context(&first).await.unwrap();
    harness
        .wait_for(|event| {
            *event
                == DriverEvent::TabDestroyed {
                    target_id: tab_one.clone(),
                }
        })
        .await;

    harness.driver.close().await.unwrap();
}

#[tokio::test]
async fn element_below_the_fold() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open(FOLD_PAGE).await;
    page.emulate(&EmulateOptions {
        viewport: Some(Size {
            width: 800,
            height: 600,
        }),
        ..EmulateOptions::default()
    })
    .await
    .unwrap();

    let clip = Clip::Element(Target::Selector("#far".to_owned()));
    let image = page
        .screenshot(&options(clip, ImageFormat::Png))
        .await
        .unwrap();

    assert_eq!(png_size(&image), (100, 50));
    assert_eq!(pixel_at(&page, &image, 50, 25).await, [255, 0, 0, 255]);

    harness.driver.close().await.unwrap();
}

#[tokio::test]
async fn captures_over_the_cap_are_refused() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open(&canvases(30)).await;

    let full = page
        .screenshot(&options(Clip::FullPage, ImageFormat::Png))
        .await;
    assert!(
        matches!(&full, Err(Error::InvalidArgument { field, .. }) if field == "area"),
        "{full:?}"
    );

    // The PDF cap is checked against a fake browser in page::capture::tests.
    harness.driver.close().await.unwrap();
}

#[tokio::test]
async fn the_device_scale_counts_toward_the_cap() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open(SCALE_PAGE).await;
    let plain = page
        .screenshot(&options(Clip::FullPage, ImageFormat::Png))
        .await;
    assert!(plain.is_ok(), "{plain:?}");

    page.emulate(&EmulateOptions {
        scale: Some(SCALE),
        ..EmulateOptions::default()
    })
    .await
    .unwrap();
    let scaled = page
        .screenshot(&options(Clip::FullPage, ImageFormat::Png))
        .await;

    assert!(
        matches!(&scaled, Err(Error::InvalidArgument { field, .. }) if field == "area"),
        "{scaled:?}"
    );

    harness.driver.close().await.unwrap();
}
