#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use brr_core::{
    Cookie, Display, DisplayEnv, Engine, Error, InstanceId, ProfileSettings, SameSite, TargetId,
    WaitUntil,
};
use brr_engine::{Driver, DriverEvent, LaunchSpec, Launched, Navigated, profile_files};
use tempfile::TempDir;
use tokio::sync::mpsc::Receiver;

const CHROME_ENV: &str = "BRR_TEST_CHROME";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const EVENT_TIMEOUT: Duration = Duration::from_secs(15);
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(10);
const PAGE_URL: &str = "data:text/html,<title>t</title>hi";
const BLANK: &str = "about:blank";
const UNRESOLVABLE_URL: &str = "http://nonexistent.invalid/";
const ATTACHMENT_BODY: &str = "saved, not shown";

pub fn chrome() -> Option<PathBuf> {
    let chrome = std::env::var_os(CHROME_ENV).map(PathBuf::from);
    if chrome.is_none() {
        eprintln!("skipped: {CHROME_ENV} is not set");
    }

    chrome
}

pub fn spec(chrome: PathBuf, dir: &Path, instance_id: InstanceId) -> LaunchSpec {
    LaunchSpec {
        instance_id,
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
        sandbox: None,
    }
}

// Chromium overwrites its own main-process environ with the rewritten command line, so the marker
// is read from every process that carries it (its children inherit it).
fn marked_processes(markers: [&str; 2]) -> usize {
    std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| std::fs::read(entry.path().join("environ")).ok())
        .filter(|environ| {
            markers.iter().all(|marker| {
                environ
                    .split(|byte| *byte == 0)
                    .any(|var| var == marker.as_bytes())
            })
        })
        .count()
}

async fn wait_for(events: &mut Receiver<DriverEvent>, matches: impl Fn(&DriverEvent) -> bool) {
    let found = tokio::time::timeout(EVENT_TIMEOUT, async {
        while let Some(event) = events.recv().await {
            if matches(&event) {
                return true;
            }
        }
        false
    })
    .await;

    assert_eq!(found, Ok(true), "expected event did not arrive");
}

async fn stop(driver: &Driver, events: &mut Receiver<DriverEvent>) {
    driver.close().await.unwrap();
    wait_for(events, |event| matches!(event, DriverEvent::Exited { .. })).await;
}

async fn open_attached(launched: &mut Launched) -> TargetId {
    let target_id = launched.driver.open_tab(BLANK).await.unwrap();
    wait_for(&mut launched.events, |event| {
        *event
            == DriverEvent::TabAttached {
                target_id: target_id.clone(),
            }
    })
    .await;

    target_id
}

#[tokio::test]
async fn launch_marks_env() {
    let Some(chrome) = chrome() else { return };
    let dir = TempDir::new().unwrap();
    let instance_id = InstanceId::new();
    let spec = spec(chrome, dir.path(), instance_id);
    let log_file = spec.log_file.clone();

    let mut launched = Driver::launch(spec).await.unwrap();

    let id_marker = format!("BRR_INSTANCE_ID={instance_id}");
    assert!(marked_processes([&id_marker, "BRR_DAEMON=1"]) > 0);
    assert!(log_file.exists());

    stop(&launched.driver, &mut launched.events).await;
}

#[tokio::test]
async fn navigate_waits_for_load_and_reports_errors() {
    let Some(chrome) = chrome() else { return };
    let dir = TempDir::new().unwrap();
    let mut launched = Driver::launch(spec(chrome, dir.path(), InstanceId::new()))
        .await
        .unwrap();
    let target_id = open_attached(&mut launched).await;

    launched
        .driver
        .navigate(&target_id, PAGE_URL, WaitUntil::Load, NAVIGATE_TIMEOUT)
        .await
        .unwrap();
    wait_for(&mut launched.events, |event| {
        matches!(event, DriverEvent::TabInfo { target_id: id, title, .. }
            if *id == target_id && title == "t")
    })
    .await;

    let error = launched
        .driver
        .navigate(
            &target_id,
            UNRESOLVABLE_URL,
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::NavigationFailed { net_error } if net_error.contains("ERR_NAME_NOT_RESOLVED"))
    );

    stop(&launched.driver, &mut launched.events).await;
}

// Answers every request with a file to save.
fn serve_attachment() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=f.bin\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{ATTACHMENT_BODY}",
                ATTACHMENT_BODY.len()
            );
        }
    });

    format!("http://{address}/file")
}

#[tokio::test]
async fn navigating_to_a_download_succeeds_and_keeps_the_page() {
    let Some(chrome) = chrome() else { return };
    let dir = TempDir::new().unwrap();
    let mut launched = Driver::launch(spec(chrome, dir.path(), InstanceId::new()))
        .await
        .unwrap();
    let target_id = open_attached(&mut launched).await;
    let file = serve_attachment();
    let loaded = launched
        .driver
        .navigate(&target_id, PAGE_URL, WaitUntil::Load, NAVIGATE_TIMEOUT)
        .await
        .unwrap();

    let navigated = launched
        .driver
        .navigate(&target_id, &file, WaitUntil::Load, NAVIGATE_TIMEOUT)
        .await
        .unwrap();

    assert_eq!(loaded, Navigated::Page);
    assert_eq!(navigated, Navigated::Download);
    wait_for(
        &mut launched.events,
        |event| matches!(event, DriverEvent::DownloadStarted { url, .. } if *url == file),
    )
    .await;
    let page = launched.driver.page(&target_id).unwrap();
    let title = page.evaluate("document.title", NAVIGATE_TIMEOUT).await;
    assert_eq!(title.unwrap().as_str(), Some("t"));

    stop(&launched.driver, &mut launched.events).await;
}

#[tokio::test]
async fn close_flow() {
    let Some(chrome) = chrome() else { return };
    let dir = TempDir::new().unwrap();
    let mut launched = Driver::launch(spec(chrome, dir.path(), InstanceId::new()))
        .await
        .unwrap();
    let target_id = open_attached(&mut launched).await;

    launched.driver.close_tab(&target_id).await.unwrap();
    wait_for(&mut launched.events, |event| {
        *event
            == DriverEvent::TabDestroyed {
                target_id: target_id.clone(),
            }
    })
    .await;

    stop(&launched.driver, &mut launched.events).await;
}

fn cookie(name: &str, value: &str, expires: f64) -> Cookie {
    Cookie {
        name: name.to_owned(),
        value: value.to_owned(),
        domain: "example.test".to_owned(),
        path: "/".to_owned(),
        expires,
        http_only: false,
        secure: false,
        same_site: SameSite::Lax,
    }
}

#[tokio::test]
async fn cookies_offline_and_cdp() {
    let Some(chrome) = chrome() else { return };
    let dir = TempDir::new().unwrap();
    let spec = spec(chrome, dir.path(), InstanceId::new());
    let user_data_dir = spec.user_data_dir.clone();

    let mut launched = Driver::launch(spec.clone()).await.unwrap();
    stop(&launched.driver, &mut launched.events).await;

    let offline = cookie("brr", "offline", 4_102_444_800.0);
    profile_files::write_cookies(&user_data_dir, std::slice::from_ref(&offline)).unwrap();

    let mut launched = Driver::launch(spec).await.unwrap();
    let cookies = launched.driver.cookies().await.unwrap();
    assert!(
        cookies.contains(&offline),
        "offline cookie missing: {}",
        cookies.len()
    );

    let live = cookie("cdp", "live", -1.0);
    assert_eq!(
        launched
            .driver
            .set_cookies(std::slice::from_ref(&live))
            .await
            .unwrap(),
        1
    );
    assert!(launched.driver.cookies().await.unwrap().contains(&live));

    stop(&launched.driver, &mut launched.events).await;
}
