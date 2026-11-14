use std::path::PathBuf;
use std::time::Duration;

use brr_cdp::{CdpEvent, connect_ws};
use tokio::process::Command;

const CHROME_ENV: &str = "BRR_TEST_CHROME";
const WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);

#[allow(clippy::disallowed_macros)]
#[tokio::test]
async fn browser_socket() {
    let Some(chrome) = std::env::var_os(CHROME_ENV).map(PathBuf::from) else {
        eprintln!("skipped: {CHROME_ENV} is not set");
        return;
    };
    let profile = tempfile::tempdir().unwrap();

    let mut command = Command::new(chrome);
    command
        .args([
            "--headless",
            "--no-first-run",
            "--no-sandbox",
            "--remote-debugging-port=0",
        ])
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .kill_on_drop(true);
    let _chrome = command.spawn().unwrap();

    let active_port = profile.path().join("DevToolsActivePort");
    let url = tokio::time::timeout(WAIT, async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&active_port) {
                let mut lines = text.lines();
                if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                    return format!("ws://127.0.0.1:{port}{path}");
                }
            }
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .unwrap();

    let (cdp, mut events) = connect_ws(&url).await.unwrap();

    assert!(!cdp.browser_version().await.unwrap().is_empty());
    cdp.set_discover_targets().await.unwrap();
    let created = tokio::time::timeout(WAIT, async {
        while let Some(event) = events.recv().await {
            if let CdpEvent::TargetCreated(_) = event {
                return true;
            }
        }
        false
    })
    .await
    .unwrap();
    assert!(created);
}
