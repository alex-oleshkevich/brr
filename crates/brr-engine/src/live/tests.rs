use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use brr_core::{Error, TargetId};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::{Notify, mpsc};

use super::{Live, Shared, devtools_active_port};
use crate::Chromium;
use crate::common::EVENTS_QUEUE;
use crate::tests::fake_cdp;

const ATTACH_METHOD: &str = "Target.attachToTarget";
const ATTACH_WAIT: Duration = Duration::from_secs(10);
const SESSION: &str = "S1";
const TARGET: &str = "T1";

fn write_port(root: &Path, dir: &str, contents: &str) {
    let dir = root.join(dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("DevToolsActivePort"), contents).unwrap();
}

#[test]
fn devtools_active_port_builds_the_browser_url() {
    let root = TempDir::new().unwrap();
    write_port(root.path(), "google-chrome", "9333\n/devtools/browser/abc");
    write_port(
        root.path(),
        "BraveSoftware/Brave-Browser",
        "9444\n/devtools/browser/b\n",
    );

    assert_eq!(
        devtools_active_port(root.path(), "chrome").unwrap(),
        "ws://127.0.0.1:9333/devtools/browser/abc"
    );
    assert_eq!(
        devtools_active_port(root.path(), "brave").unwrap(),
        "ws://127.0.0.1:9444/devtools/browser/b"
    );
}

#[test]
fn devtools_active_port_rejects_unknown_name() {
    let root = TempDir::new().unwrap();

    assert_eq!(
        devtools_active_port(root.path(), "opera").unwrap_err(),
        Error::InvalidArgument {
            field: "install_name".to_owned(),
            message: "expected one of chrome, chrome-beta, chromium, brave, edge".to_owned(),
        }
    );
}

#[test]
fn devtools_active_port_missing_file() {
    let root = TempDir::new().unwrap();

    assert_eq!(
        devtools_active_port(root.path(), "edge").unwrap_err(),
        Error::DependencyMissing {
            name: "DevToolsActivePort".to_owned(),
            help: "enable remote debugging at chrome://inspect/#remote-debugging in edge, \
                   then attach again"
                .to_owned(),
        }
    );
}

#[tokio::test]
async fn a_cancelled_attach_is_not_attached_again() {
    let (cdp, cdp_events, mut peer) = fake_cdp();
    let (events_tx, _events) = mpsc::channel(EVENTS_QUEUE);
    let stop = Arc::new(Notify::new());
    let shared = Arc::new(Shared::default());
    let chromium = Chromium::connect(
        cdp,
        cdp_events,
        events_tx,
        stop.clone(),
        Vec::new(),
        shared.clone(),
    );
    let live = Live {
        chromium,
        shared,
        stop,
    };
    let target_id = TargetId(TARGET.to_owned());

    let first = tokio::spawn({
        let (live, target_id) = (live.clone(), target_id.clone());
        async move { live.attach_tab(&target_id).await }
    });
    let request = peer.read().await;
    assert_eq!(request["method"], ATTACH_METHOD);
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());

    let attaches = Arc::new(AtomicUsize::new(1));
    let counted = attaches.clone();
    tokio::spawn(async move {
        peer.answer(&request, json!({ "sessionId": SESSION })).await;
        loop {
            let request = peer.read().await;
            if request["method"] == ATTACH_METHOD {
                counted.fetch_add(1, Ordering::SeqCst);
            }
            peer.answer(&request, json!({})).await;
        }
    });

    tokio::time::timeout(ATTACH_WAIT, live.attach_tab(&target_id))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(attaches.load(Ordering::SeqCst), 1);
    assert_eq!(live.chromium.session_ids().len(), 1);
}
