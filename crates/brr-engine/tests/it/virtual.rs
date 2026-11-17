#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use brr_core::{Display, DisplayEnv, Engine, InstanceId, ProfileSettings, Size, WaitUntil};
use brr_engine::{Driver, LaunchSpec, Launched};
use serde_json::json;
use tempfile::TempDir;

use crate::common;

use common::{exists, has_xvfb, processes_with};

const CHROME_ENV: &str = "BRR_TEST_CHROME";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(15);
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);
const GONE_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SCREEN: Size = Size {
    width: 1024,
    height: 768,
};

fn chrome() -> Option<PathBuf> {
    let chrome = std::env::var_os(CHROME_ENV).map(PathBuf::from);
    if chrome.is_none() {
        eprintln!("skipped: {CHROME_ENV} is not set");
    }

    chrome
}

fn spec(chrome: PathBuf, dir: &Path, instance_id: InstanceId) -> LaunchSpec {
    LaunchSpec {
        instance_id,
        engine: Engine::Chromium,
        executable: chrome,
        user_data_dir: dir.join("profile"),
        log_file: dir.join("logs").join("browser.log"),
        display: Display::Virtual,
        display_env: DisplayEnv::default(),
        settings: ProfileSettings {
            screen: Some(SCREEN),
            ..ProfileSettings::default()
        },
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

#[tokio::test]
async fn chromium_runs_on_a_virtual_display() {
    let Some(chrome) = chrome() else { return };
    if !has_xvfb() {
        return;
    }
    let dir = TempDir::new().unwrap();
    let instance_id = InstanceId::new();
    let Launched {
        driver,
        mut events,
        first_target_id,
        xvfb_pid,
        ..
    } = Driver::launch(spec(chrome, dir.path(), instance_id))
        .await
        .unwrap();
    tokio::spawn(async move { while events.recv().await.is_some() {} });
    let xvfb_pid = xvfb_pid.unwrap();

    let id_marker = format!("BRR_INSTANCE_ID={instance_id}");
    assert!(processes_with(&[&id_marker, "DISPLAY=:", "XAUTHORITY="]) > 0);
    assert_eq!(processes_with(&[&id_marker, "WAYLAND_DISPLAY="]), 0);

    driver
        .navigate(
            &first_target_id,
            "data:text/html,<p>x</p>",
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await
        .unwrap();
    let page = driver.page(&first_target_id).unwrap();
    let width = page.evaluate("screen.width", EVAL_TIMEOUT).await.unwrap();
    assert_eq!(width, json!(SCREEN.width));

    driver.close().await.unwrap();
    tokio::time::timeout(GONE_TIMEOUT, async {
        while exists(xvfb_pid) {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .unwrap();
}
