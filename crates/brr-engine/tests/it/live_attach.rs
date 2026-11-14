use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use brr_cdp::Cdp;
use brr_core::{InstanceId, TargetId};
use brr_engine::{AttachSpec, Attached, Connection, Driver, DriverEvent, devtools_active_port};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::mpsc::Receiver;

const CHROME_ENV: &str = "BRR_TEST_CHROME";
const EVENT_TIMEOUT: Duration = Duration::from_secs(15);
const PAGE_ONE: &str = "data:text/html,<title>one</title>one";
const PAGE_TWO: &str = "data:text/html,<title>two</title>two";
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(10);
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const PORT_FILE_TIMEOUT: Duration = Duration::from_secs(20);
const PAGES_TIMEOUT: Duration = Duration::from_secs(15);

fn chrome() -> Option<PathBuf> {
    let chrome = std::env::var_os(CHROME_ENV).map(PathBuf::from);
    if chrome.is_none() {
        eprintln!("skipped: {CHROME_ENV} is not set");
    }

    chrome
}

async fn wait_for(events: &mut Receiver<DriverEvent>, expected: DriverEvent) {
    let found = tokio::time::timeout(EVENT_TIMEOUT, async {
        while let Some(event) = events.recv().await {
            if event == expected {
                return true;
            }
        }
        false
    })
    .await;

    assert_eq!(found, Ok(true), "expected event did not arrive");
}

struct Browser {
    child: Child,
    ws_url: String,
    _dir: TempDir,
}

impl Browser {
    async fn start(chrome: &Path) -> Browser {
        let dir = TempDir::new().unwrap();
        let config_dir = dir.path().join("google-chrome");
        let child = Command::new(chrome)
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", config_dir.display()))
            .args(["--headless", "--no-first-run", "--no-default-browser-check"])
            .arg(PAGE_ONE)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let ws_url = tokio::time::timeout(PORT_FILE_TIMEOUT, async {
            loop {
                if let Ok(url) = devtools_active_port(dir.path(), "chrome") {
                    return url;
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        })
        .await
        .unwrap();

        // Headless Chromium takes one start URL, so the second page comes from a short connection.
        let (cdp, _events): (Cdp, _) = brr_cdp::connect_ws(&ws_url).await.unwrap();
        cdp.create_target(PAGE_TWO, false).await.unwrap();

        // Chromium lists a page before its start URL commits; attaching then sees about:blank.
        tokio::time::timeout(PAGES_TIMEOUT, async {
            loop {
                let result = cdp
                    .call(None, "Target.getTargets", json!({}))
                    .await
                    .unwrap();
                let infos = result["targetInfos"].as_array().unwrap();
                if [PAGE_ONE, PAGE_TWO]
                    .iter()
                    .all(|url| infos.iter().any(|info| info["url"] == *url))
                {
                    return;
                }

                tokio::time::sleep(POLL_INTERVAL).await;
            }
        })
        .await
        .expect("both start pages commit");

        Browser {
            child,
            ws_url,
            _dir: dir,
        }
    }

    async fn attach(&self) -> Attached {
        Driver::attach(AttachSpec {
            instance_id: InstanceId::new(),
            connection: Connection::WebSocket(self.ws_url.clone()),
        })
        .await
        .unwrap()
    }

    // Target.getTargets from a second connection: which pages have a client attached.
    async fn attached_urls(&self) -> Vec<String> {
        let (cdp, _events): (Cdp, _) = brr_cdp::connect_ws(&self.ws_url).await.unwrap();
        let result = cdp
            .call(None, "Target.getTargets", json!({}))
            .await
            .unwrap();
        let infos = result["targetInfos"].as_array().unwrap().clone();

        infos
            .iter()
            .filter(|info| info["type"] == "page" && info["attached"] == true)
            .map(|info| info["url"].as_str().unwrap().to_owned())
            .collect()
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn target_with(attached: &Attached, url: &str) -> TargetId {
    attached
        .targets
        .iter()
        .find(|target| target.url == url)
        .unwrap()
        .target_id
        .clone()
}

#[tokio::test]
async fn discover_only() {
    let Some(chrome) = chrome() else { return };
    let browser = Browser::start(&chrome).await;

    let mut attached = browser.attach().await;

    let urls: Vec<&str> = attached.targets.iter().map(|t| t.url.as_str()).collect();
    assert_eq!(urls.len(), 2, "{urls:?}");
    assert!(urls.contains(&PAGE_ONE) && urls.contains(&PAGE_TWO));
    assert!(attached.driver.is_live());
    assert_eq!(browser.attached_urls().await, Vec::<String>::new());

    let first = target_with(&attached, PAGE_ONE);
    wait_for(
        &mut attached.events,
        DriverEvent::TabLoaded { target_id: first },
    )
    .await;
}

#[tokio::test]
async fn lazy_attach() {
    let Some(chrome) = chrome() else { return };
    let browser = Browser::start(&chrome).await;
    let attached = browser.attach().await;
    let first = target_with(&attached, PAGE_ONE);

    attached.driver.attach_tab(&first).await.unwrap();
    let page = attached.driver.page(&first).unwrap();

    let value: Value = page.evaluate("1+1", EVAL_TIMEOUT).await.unwrap();
    assert_eq!(value, json!(2));
    assert_eq!(browser.attached_urls().await, vec![PAGE_ONE.to_owned()]);

    let target = "data:text/html,<title>nav</title>nav";
    attached
        .driver
        .navigate(&first, target, brr_core::WaitUntil::Load, NAVIGATE_TIMEOUT)
        .await
        .unwrap();
    let href = page.evaluate("location.href", EVAL_TIMEOUT).await.unwrap();
    assert_eq!(href, json!(target));
}

#[tokio::test]
async fn detach_never_kills() {
    let Some(chrome) = chrome() else { return };
    let mut browser = Browser::start(&chrome).await;
    let attached = browser.attach().await;
    let first = target_with(&attached, PAGE_ONE);
    attached.driver.attach_tab(&first).await.unwrap();

    attached.driver.detach().await.unwrap();

    assert!(browser.child.try_wait().unwrap().is_none());
    assert_eq!(browser.attached_urls().await, Vec::<String>::new());
    let Attached {
        driver,
        mut events,
        targets,
        ..
    } = browser.attach().await;
    assert!(driver.close().await.is_err());
    assert_eq!(targets.len(), 2);

    browser.child.kill().unwrap();
    wait_for(
        &mut events,
        DriverEvent::Exited {
            code: None,
            signal: None,
        },
    )
    .await;
}
