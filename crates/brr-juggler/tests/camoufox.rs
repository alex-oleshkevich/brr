#![allow(clippy::expect_used, clippy::disallowed_macros)]

use std::path::PathBuf;
use std::time::Duration;

use brr_juggler::{Juggler, JugglerEvent};
use brr_proto::spawn;
use serde_json::{Map, Value};
use tokio::process::Command;
use tokio::sync::mpsc::Receiver;

const CAMOUFOX_ENV: &str = "BRR_TEST_CAMOUFOX";
const TIMEOUT: Duration = Duration::from_secs(60);
const URL: &str = "data:text/html,hi";

async fn next_event(events: &mut Receiver<JugglerEvent>) -> JugglerEvent {
    tokio::time::timeout(TIMEOUT, events.recv())
        .await
        .expect("timed out waiting for a Juggler event")
        .expect("Juggler event stream ended")
}

#[tokio::test]
async fn camoufox_pages_attach_and_navigate() {
    let Some(binary) = std::env::var_os(CAMOUFOX_ENV).map(PathBuf::from) else {
        eprintln!("skipped: {CAMOUFOX_ENV} is not set");
        return;
    };
    let profile = tempfile::tempdir().unwrap();

    let mut command = Command::new(binary);
    command
        .args(["-juggler-pipe", "-headless", "-no-remote", "-profile"])
        .arg(profile.path())
        .kill_on_drop(true);
    let mut spawned = spawn(command).unwrap();
    let (juggler, mut events) = Juggler::start(spawned.mux.clone(), spawned.events);

    juggler.enable(&Map::new()).await.unwrap();
    assert!(!juggler.browser_version().await.unwrap().is_empty());

    let target = juggler.new_page(None).await.unwrap();
    let session = loop {
        if let JugglerEvent::AttachedToTarget {
            session_id,
            target_id,
            ..
        } = next_event(&mut events).await
            && target_id == target
        {
            break session_id;
        }
    };

    // Juggler reports the main frame as the Page.frameAttached without a parentFrameId.
    let frame_id = loop {
        if let JugglerEvent::Other {
            method,
            params,
            session_id,
        } = next_event(&mut events).await
            && method == "Page.frameAttached"
            && session_id.as_ref() == Some(&session)
            && params.get("parentFrameId").is_none_or(Value::is_null)
        {
            break params["frameId"].as_str().unwrap().to_owned();
        }
    };

    let load = juggler.wait_for(
        Some(&session),
        "Page.eventFired",
        Box::new(|params| params["name"] == "load"),
    );
    // Events must keep draining while the navigation is awaited.
    tokio::spawn(async move { while events.recv().await.is_some() {} });
    juggler.navigate(&session, &frame_id, URL).await.unwrap();
    tokio::time::timeout(TIMEOUT, load).await.unwrap().unwrap();

    juggler.close_page(&session).await.unwrap();
    juggler.close_browser().await.unwrap();
    let status = tokio::time::timeout(TIMEOUT, spawned.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success(), "{status}");
}
