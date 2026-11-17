use std::ffi::OsString;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use brr_cdp::{Cdp, SessionId};
use brr_core::{Display, ProfileSettings, Size, TargetId, WebRtcPolicy};
use serde_json::{Value, json};
use test_case::test_case;
use tokio::sync::{Notify, mpsc, oneshot};

use super::session::register_tab;
use super::targets::{Mode, run};
use super::{Chromium, Inner, args};
use crate::DriverEvent;
use crate::common::EVENTS_QUEUE;
use crate::live::Shared;
use crate::page::EmulateOptions;
use crate::tests::{Peer, fake_cdp, spec};

const SCREEN_WIDTH: u32 = 800;
const SCREEN_HEIGHT: u32 = 600;
const EVENT_WAIT: Duration = Duration::from_secs(10);
const FLOOD_EVENTS: usize = 4 * EVENTS_QUEUE;
const SESSION: &str = "S1";
const TARGET: &str = "T1";
const WORKER: &str = "W1";
const PAGE_URL: &str = "https://example.test/";
const PAGE_TITLE: &str = "Example";

fn os(flag: &str) -> OsString {
    OsString::from(flag)
}

#[test]
fn args_lists_flags_in_order() {
    let settings = ProfileSettings {
        screen: Some(Size {
            width: SCREEN_WIDTH,
            height: SCREEN_HEIGHT,
        }),
        locale: Some("de-DE".to_owned()),
        args: vec!["--mute-audio".to_owned()],
        ..ProfileSettings::default()
    };
    let mut spec = spec(Display::Headless, settings);
    spec.proxy_server = Some("http://proxy:8080".to_owned());

    let args = args(&spec, false);

    let expected = [
        "--remote-debugging-pipe",
        "--password-store=basic",
        "--enable-unsafe-extension-debugging",
        "--user-data-dir=/tmp/profile",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-backgrounding-occluded-windows",
        "--disable-renderer-backgrounding",
        "--disable-background-timer-throttling",
        "--headless",
        "--window-size=800,600",
        "--lang=de-DE",
        "--proxy-server=http://proxy:8080",
        "--proxy-bypass-list=<-loopback>",
        "--mute-audio",
        "about:blank",
    ];
    assert_eq!(args, expected.map(os));
}

#[test]
fn container_shm_flag() {
    let spec = spec(Display::Headless, ProfileSettings::default());

    assert!(args(&spec, true).contains(&os("--disable-dev-shm-usage")));
    assert!(!args(&spec, false).contains(&os("--disable-dev-shm-usage")));
}

#[test_case(Display::Headless, "--headless", true)]
#[test_case(Display::Visible, "--headless", false)]
#[test_case(Display::Virtual, "--ozone-platform=x11", true)]
#[test_case(Display::Headless, "--ozone-platform=x11", false)]
fn args_follow_display(display: Display, flag: &str, present: bool) {
    let spec = spec(display, ProfileSettings::default());

    assert_eq!(args(&spec, false).contains(&os(flag)), present);
}

#[test]
fn args_m3_webrtc_and_loopback() {
    let settings = ProfileSettings {
        webrtc: WebRtcPolicy::DisableNonProxiedUdp,
        ..ProfileSettings::default()
    };
    let mut spec = spec(Display::Headless, settings);
    spec.proxy_server = Some("http://127.0.0.1:9".to_owned());

    let proxied = args(&spec, false);

    assert!(proxied.contains(&os(
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp"
    )));
    assert!(proxied.contains(&os("--proxy-bypass-list=<-loopback>")));

    let plain = args(
        &crate::tests::spec(Display::Headless, ProfileSettings::default()),
        false,
    );

    assert!(
        !plain
            .iter()
            .any(|arg| arg.to_string_lossy().starts_with("--force-webrtc")
                || arg.to_string_lossy().starts_with("--proxy-bypass"))
    );
}

#[test_case("big file?.bin", "big file_.bin"; "unsafe characters")]
#[test_case("..", "download"; "parent segment")]
#[test_case(".", "download"; "current segment")]
#[test_case("", "download"; "empty")]
#[test_case("a..b", "a..b"; "dots inside")]
fn download_names_are_safe_path_segments(name: &str, expected: &str) {
    assert_eq!(super::downloads::sanitize(name), expected);
}

fn inner(cdp: Cdp) -> Arc<Inner> {
    Arc::new(Inner {
        cdp,
        download_dir: "/tmp/downloads".into(),
        console_buffer: 10,
        network_buffer: 10,
        profile_view: EmulateOptions::default(),
        content_defaults: Mutex::default(),
        tabs: Mutex::default(),
        routes: Mutex::default(),
    })
}

fn target_id(id: &str) -> TargetId {
    TargetId(id.to_owned())
}

fn target_info(id: &str, kind: &str) -> Value {
    json!({
        "targetInfo": {
            "targetId": id,
            "type": kind,
            "title": PAGE_TITLE,
            "url": PAGE_URL,
            "attached": false,
            "canAccessOpener": false,
        }
    })
}

async fn next(events: &mut mpsc::Receiver<DriverEvent>) -> DriverEvent {
    tokio::time::timeout(EVENT_WAIT, events.recv())
        .await
        .unwrap()
        .unwrap()
}

// Sends more events than the dispatcher and its consumer can queue: the peer only gets through
// them all when the target loop keeps consuming.
async fn flood(peer: &mut Peer) {
    for _ in 0..FLOOD_EVENTS {
        peer.event(
            "Target.targetInfoChanged",
            target_info(WORKER, "worker"),
            None,
        )
        .await;
    }
}

#[tokio::test]
async fn pending_title_read_does_not_stall_the_event_stream() {
    let (cdp, cdp_events, mut peer) = fake_cdp();
    let inner = inner(cdp);
    register_tab(&inner, &SessionId(SESSION.to_owned()), &target_id(TARGET));
    let (events_tx, mut events) = mpsc::channel(EVENTS_QUEUE);
    let (ready, _ready_rx) = oneshot::channel();
    tokio::spawn(run(inner, cdp_events, events_tx, Mode::Launch(ready)));

    peer.event(
        "Page.loadEventFired",
        json!({ "timestamp": 1.0 }),
        Some(SESSION),
    )
    .await;
    let request = peer.read().await;
    assert_eq!(request["method"], "Target.getTargetInfo");

    tokio::time::timeout(EVENT_WAIT, flood(&mut peer))
        .await
        .unwrap();
    peer.answer(&request, target_info(TARGET, "page")).await;

    assert_eq!(
        next(&mut events).await,
        DriverEvent::TabLoaded {
            target_id: target_id(TARGET)
        }
    );
    assert_eq!(
        next(&mut events).await,
        DriverEvent::TabInfo {
            target_id: target_id(TARGET),
            url: PAGE_URL.to_owned(),
            title: PAGE_TITLE.to_owned(),
        }
    );
}

#[tokio::test]
async fn held_creation_lock_does_not_stall_the_event_stream() {
    let (cdp, cdp_events, mut peer) = fake_cdp();
    let (events_tx, mut events) = mpsc::channel(EVENTS_QUEUE);
    let shared = Arc::new(Shared::default());
    let creating = shared.creating().await;
    let _chromium = Chromium::connect(
        cdp,
        cdp_events,
        events_tx,
        Arc::new(Notify::new()),
        Vec::new(),
        shared.clone(),
    );

    peer.event("Target.targetCreated", target_info(TARGET, "page"), None)
        .await;
    tokio::time::timeout(EVENT_WAIT, flood(&mut peer))
        .await
        .unwrap();
    drop(creating);

    assert_eq!(
        next(&mut events).await,
        DriverEvent::TabCreated {
            target_id: target_id(TARGET),
            url: PAGE_URL.to_owned(),
        }
    );
}

#[tokio::test]
async fn only_pages_report_destroyed() {
    let (cdp, cdp_events, mut peer) = fake_cdp();
    let (events_tx, mut events) = mpsc::channel(EVENTS_QUEUE);
    let (ready, _ready_rx) = oneshot::channel();
    tokio::spawn(run(inner(cdp), cdp_events, events_tx, Mode::Launch(ready)));

    let gone = |id: &str| json!({ "targetId": id });
    peer.event("Target.targetDestroyed", gone(WORKER), None)
        .await;
    peer.event("Target.targetCrashed", gone(WORKER), None).await;
    peer.event("Target.targetCreated", target_info(TARGET, "page"), None)
        .await;
    peer.event("Target.targetCrashed", gone(TARGET), None).await;
    peer.event("Target.targetDestroyed", gone(TARGET), None)
        .await;

    for expected in [
        DriverEvent::TabCreated {
            target_id: target_id(TARGET),
            url: PAGE_URL.to_owned(),
        },
        DriverEvent::TabCrashed {
            target_id: target_id(TARGET),
        },
        DriverEvent::TabDestroyed {
            target_id: target_id(TARGET),
        },
    ] {
        assert_eq!(next(&mut events).await, expected);
    }
}
