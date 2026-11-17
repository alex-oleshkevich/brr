#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use brr_core::{
    Display, DisplayEnv, Engine, Error, InstanceId, ProfileSettings, TargetId, WaitUntil,
};
use brr_engine::page::{Link, Page, Target};
use brr_engine::{Driver, LaunchSpec};
use serde_json::json;
use tempfile::TempDir;

const CHROME_ENV: &str = "BRR_TEST_CHROME";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(10);
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
const MISSING_TIMEOUT: Duration = Duration::from_millis(300);
const FRAMED_URL: &str =
    "data:text/html,<button>Go</button><iframe srcdoc=\"<a href='/x'>In</a>\"></iframe>";
const TWO_BUTTONS_URL: &str = "data:text/html,<button>One</button><button>Two</button>";
const OTHER_URL: &str = "data:text/html,<p>other</p>";
const READ_URL: &str = "data:text/html,<!DOCTYPE html><html lang='de'><head><base href='http://example.test/'><title>T</title><meta name='description' content='D'><meta property='og:title' content='OG'><link rel='canonical' href='/canon'></head><body>Visible text<a href='/rel' rel='nofollow'>Rel link</a><a href='/rel'>Same</a><a href='https://example.com/abs'>Abs</a></body></html>";

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
        sandbox: None,
    }
}

struct Harness {
    driver: Driver,
    target_id: TargetId,
    _dir: TempDir,
}

impl Harness {
    async fn start(chrome: PathBuf) -> Harness {
        let dir = TempDir::new().unwrap();
        let launched = Driver::launch(spec(chrome, dir.path())).await.unwrap();
        let mut events = launched.events;
        tokio::spawn(async move { while events.recv().await.is_some() {} });

        Harness {
            driver: launched.driver,
            target_id: launched.first_target_id,
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

    async fn stop(self) {
        self.driver.close().await.unwrap();
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

#[tokio::test]
async fn snapshot_refs_and_iframes() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open(FRAMED_URL).await;

    let snapshot = page.snapshot().await.unwrap();

    assert!(snapshot.contains("button \"Go\" [ref=e"), "{snapshot}");
    let frame_line = snapshot
        .lines()
        .position(|line| line.contains("- iframe"))
        .expect("iframe line");
    let link_line = snapshot.lines().nth(frame_line + 1).unwrap();
    assert!(link_line.contains("link \"In\" [ref=f"), "{snapshot}");

    let link = ref_of(&snapshot, "link \"In\"");
    page.resolve(&Target::Ref(link), RESOLVE_TIMEOUT)
        .await
        .unwrap();

    harness.stop().await;
}

#[tokio::test]
async fn stale_and_missing() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open(TWO_BUTTONS_URL).await;

    let first = page.snapshot().await.unwrap();
    let stale = ref_of(&first, "button \"Two\"");
    page.resolve(&Target::Ref(stale.clone()), RESOLVE_TIMEOUT)
        .await
        .unwrap();

    let second = page.snapshot().await.unwrap();
    let still = page
        .resolve(&Target::Ref(stale.clone()), RESOLVE_TIMEOUT)
        .await;
    assert_eq!(still.is_ok(), second.contains(&format!("[ref={stale}]")));

    let missing = page
        .resolve(&Target::Selector("#missing".to_owned()), MISSING_TIMEOUT)
        .await;
    assert_eq!(missing, Err(Error::ElementNotFound));

    let ambiguous = page
        .resolve(&Target::Selector("button".to_owned()), RESOLVE_TIMEOUT)
        .await;
    assert!(matches!(
        ambiguous,
        Err(Error::InvalidArgument { field, .. }) if field == "selector"
    ));
    page.resolve(
        &Target::Selector("button >> text=One".to_owned()),
        RESOLVE_TIMEOUT,
    )
    .await
    .unwrap();

    // Two navigations, a snapshot after each: the utility world is rebuilt per document.
    let page = harness.open(OTHER_URL).await;
    assert_eq!(
        page.resolve(&Target::Ref(stale), RESOLVE_TIMEOUT).await,
        Err(Error::StaleRef)
    );
    assert!(page.snapshot().await.unwrap().contains("other"));
    let page = harness.open(TWO_BUTTONS_URL).await;
    assert!(page.snapshot().await.unwrap().contains("button \"One\""));

    harness.stop().await;
}

#[tokio::test]
async fn read_tools() {
    let Some(chrome) = chrome() else { return };
    let harness = Harness::start(chrome).await;
    let page = harness.open(READ_URL).await;

    assert!(page.html().await.unwrap().starts_with("<!DOCTYPE html>"));
    assert!(page.text().await.unwrap().contains("Visible text"));

    let links = page.links().await.unwrap();
    let hrefs: Vec<&str> = links.iter().map(|link: &Link| link.href.as_str()).collect();
    assert_eq!(hrefs.len(), 2, "{links:?}");
    assert_eq!(
        hrefs,
        ["http://example.test/rel", "https://example.com/abs"]
    );
    assert_eq!(links[0].text, "Rel link");
    assert_eq!(links[0].rel, "nofollow");

    let metadata = page.metadata().await.unwrap();
    assert_eq!(metadata.title, "T");
    assert_eq!(metadata.description.as_deref(), Some("D"));
    assert_eq!(metadata.lang.as_deref(), Some("de"));
    assert_eq!(
        metadata.canonical.as_deref(),
        Some("http://example.test/canon")
    );
    assert_eq!(metadata.og.get("og:title").map(String::as_str), Some("OG"));

    assert_eq!(
        page.evaluate("1+1", Duration::from_secs(5)).await.unwrap(),
        json!(2)
    );
    let failed = page
        .evaluate("throw new Error('x')", Duration::from_secs(5))
        .await;
    assert!(matches!(failed, Err(Error::EvaluationFailed { message }) if message.contains('x')));

    harness.stop().await;
}
