#![allow(clippy::unwrap_used)]

use crate::common;

use std::{sync::Arc, time::Duration};

use brr_core::{
    AdmissionMode, ClientId, ClientKind, EndReason, Error, IdleOverrides, Instance, InstanceId,
    InstanceState, Profile, TabCloseReason, TabId, now_ms,
};
use brr_daemon::{App, CallClient, ops, services::instances::LaunchRequest};
use brr_store::tab;
use serde_json::{Value, json};
use tokio::time;

use common::{KillOnDrop, add_profile, chrome_app, ctx, row, triggers, wait_for};

const LONG_GRACE_S: u32 = 600;
const MIN_IDLE_MIN: u32 = 1;
const REAIM_TIMEOUT: Duration = Duration::from_secs(90);
const REAIM_POLL: Duration = Duration::from_millis(250);

fn client(label: &str) -> CallClient {
    CallClient {
        client_id: ClientId::new(),
        kind: ClientKind::Mcp,
        label: label.to_owned(),
        implicit: true,
    }
}

async fn call(
    app: &Arc<App>,
    client: &CallClient,
    name: &str,
    input: Value,
) -> Result<Value, Error> {
    let mut context = ctx(app);
    context.client = client.clone();

    ops::dispatch(context, ops::find(name).unwrap(), input).await
}

// instance_min = 0 makes the Instance idle as soon as it runs, so a test needs no real minutes.
fn idles_at_once() -> IdleOverrides {
    IdleOverrides {
        instance_min: Some(0),
        grace_s: Some(LONG_GRACE_S),
        ..IdleOverrides::default()
    }
}

async fn launch(app: &Arc<App>, profile: &Profile) -> (Instance, brr_core::Tab) {
    app.instances
        .launch(
            app,
            LaunchRequest {
                profile: profile.clone(),
                display: None,
                priority: 5,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap()
}

fn view_of(
    app: &Arc<App>,
    instance_id: InstanceId,
) -> brr_daemon::services::instances::InstanceView {
    app.instances
        .views()
        .into_iter()
        .find(|view| view.instance.id == instance_id)
        .unwrap()
}

// A command round trip returns only after the owner drained earlier activity; the second one
// also waits for the view the first one published.
async fn owner_settled(app: &Arc<App>, instance_id: InstanceId, tab_id: &TabId) {
    for _ in 0..2 {
        app.instances
            .set_tab_pinned(&instance_id, tab_id, false)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn guard_records_activity() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, "guard", false, IdleOverrides::default()).await;
    let (running, first) = launch(&app, &profile).await;
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    let a = client("a");

    let before = view_of(&app, running.id);
    wait_for("the clock to advance", async || {
        now_ms() > before.instance.last_activity_at
    })
    .await;

    // Read-only calls never count (LC4).
    app.leases
        .guard(&app, &a, &running.id, Some(&first.id), true)
        .await
        .unwrap();
    owner_settled(&app, running.id, &first.id).await;
    let after_read = view_of(&app, running.id);
    assert_eq!(
        after_read.instance.last_activity_at,
        before.instance.last_activity_at
    );
    assert_eq!(
        after_read.tabs[0].last_activity_at,
        before.tabs[0].last_activity_at
    );

    app.leases
        .guard(&app, &a, &running.id, Some(&first.id), false)
        .await
        .unwrap();
    owner_settled(&app, running.id, &first.id).await;
    let after_write = view_of(&app, running.id);
    assert!(after_write.instance.last_activity_at > before.instance.last_activity_at);
    assert!(after_write.tabs[0].last_activity_at > before.tabs[0].last_activity_at);
}

#[tokio::test]
async fn pin_unpin() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, "pin", false, IdleOverrides::default()).await;
    let (running, _) = launch(&app, &profile).await;
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    let a = client("a");
    let b = client("b");

    let opened = call(
        &app,
        &a,
        "tab.open",
        json!({ "instance_id": running.id.to_string() }),
    )
    .await
    .unwrap();
    let tab_id = opened["tab"]["id"].as_str().unwrap().to_owned();

    let pinned = call(&app, &a, "tab.pin", json!({ "tab_id": tab_id }))
        .await
        .unwrap();
    assert_eq!(pinned["pinned"], true);
    let got = call(&app, &a, "tab.get", json!({ "tab_id": tab_id }))
        .await
        .unwrap();
    assert_eq!(got["pinned"], true);
    let id = tab_id.parse().unwrap();
    let stored = app.store.run(move |c| tab::get(c, &id)).await.unwrap();
    assert!(stored.pinned);

    let refused = call(&app, &b, "tab.unpin", json!({ "tab_id": tab_id })).await;
    assert!(matches!(refused, Err(Error::LeaseHeld { .. })));

    let unpinned = call(&app, &a, "tab.unpin", json!({ "tab_id": tab_id }))
        .await
        .unwrap();
    assert_eq!(unpinned["pinned"], false);
}

#[tokio::test]
async fn evict_only_from_idle_or_reaping() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };

    // Pinned: idle and never reaped.
    let pinned = add_profile(&app, build_id, "pinned", true, idles_at_once()).await;
    let (idle, _) = launch(&app, &pinned).await;
    // Unpinned: idle, then reaping at once.
    let unpinned = add_profile(&app, build_id, "unpinned", false, idles_at_once()).await;
    let (reaping, _) = launch(&app, &unpinned).await;
    // Default timeouts: still running.
    let plain = add_profile(&app, build_id, "plain", false, IdleOverrides::default()).await;
    let (running, _) = launch(&app, &plain).await;
    let _guard = KillOnDrop::new(&app, vec![idle.id, reaping.id, running.id]);

    wait_for("idle and reaping", async || {
        row(&app, idle.id).await.state == InstanceState::Idle
            && row(&app, reaping.id).await.state == InstanceState::Reaping
    })
    .await;
    assert_eq!(
        view_of(&app, reaping.id).instance.state,
        InstanceState::Reaping
    );

    let refused = app
        .instances
        .stop(&app, &running.id, EndReason::Evicted)
        .await;
    assert!(matches!(refused, Err(Error::InvalidTransition { .. })));
    assert_eq!(row(&app, running.id).await.state, InstanceState::Running);

    // The Profile is pinned: it idles but is never picked, even when a stale pick arrives.
    let spared = app.instances.stop(&app, &idle.id, EndReason::Evicted).await;
    assert!(matches!(spared, Err(Error::InvalidTransition { .. })));
    assert_eq!(row(&app, idle.id).await.state, InstanceState::Idle);
    let pressure = app
        .instances
        .stop(&app, &idle.id, EndReason::MemoryPressure)
        .await;
    assert!(matches!(pressure, Err(Error::InvalidTransition { .. })));

    let cut_short = app
        .instances
        .stop(&app, &reaping.id, EndReason::Evicted)
        .await
        .unwrap();
    assert_eq!(cut_short.end_reason, Some(EndReason::Evicted));
    let seen = triggers(&app, reaping.id).await;
    assert!(seen.contains(&"reap".to_owned()));
    assert!(seen.contains(&"grace_over".to_owned()));

    for instance_id in [running.id, idle.id] {
        app.instances
            .stop(&app, &instance_id, EndReason::Stop)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn profile_update_applies_to_running_instances() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, "live", false, IdleOverrides::default()).await;
    let (running, _) = launch(&app, &profile).await;
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    let a = client("a");
    assert_eq!(row(&app, running.id).await.state, InstanceState::Running);

    // Shorter idle timeout, as a stored override: the running owner re-aims at once. The op
    // refuses 0 minutes, so the row is written directly.
    let mut stored = profile.clone();
    stored.settings.idle = idles_at_once();
    let to_store = stored.clone();
    app.store
        .run(move |c| brr_store::profile::update(c, &to_store))
        .await
        .unwrap();
    app.instances
        .profile_changed(&stored.id, stored.pinned, stored.settings.idle.clone())
        .await;
    wait_for("the instance to start reaping", async || {
        row(&app, running.id).await.state == InstanceState::Reaping
    })
    .await;

    // Pinning cancels the reaping: back to running through activity, then idle for good.
    call(
        &app,
        &a,
        "profile.update",
        json!({ "profile": profile.id.to_string(), "pinned": true }),
    )
    .await
    .unwrap();
    wait_for("the pinned instance to idle", async || {
        triggers(&app, running.id).await.last().map(String::as_str) == Some("idle_timeout")
    })
    .await;
    assert_eq!(row(&app, running.id).await.state, InstanceState::Idle);
    assert!(
        triggers(&app, running.id)
            .await
            .contains(&"activity".to_owned())
    );
}

#[tokio::test]
async fn settings_set_reaims_the_timer() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, "reaim", false, IdleOverrides::default()).await;
    let (running, _) = launch(&app, &profile).await;
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    assert_eq!(row(&app, running.id).await.state, InstanceState::Running);

    // 30 minutes down to the 1-minute minimum: the wait that is already running must be re-aimed,
    // and the deadline counts from the Instance's last activity.
    app.set_settings([("idle.instance_min".to_owned(), json!(MIN_IDLE_MIN))].into())
        .await
        .unwrap();
    app.instances.recompute_idle().await;

    let idled = time::timeout(REAIM_TIMEOUT, async {
        while row(&app, running.id).await.state == InstanceState::Running {
            time::sleep(REAIM_POLL).await;
        }
    })
    .await;
    assert!(idled.is_ok(), "not idle within {REAIM_TIMEOUT:?}");
}

// The pressure Reaper decides from a snapshot; a Tab that is in use is not its to close.
#[tokio::test]
async fn reaped_close_refuses_an_active_tab() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, "active", false, IdleOverrides::default()).await;
    let (running, first) = launch(&app, &profile).await;
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    let refused = app
        .instances
        .close_tab(&running.id, &first.id, TabCloseReason::Reaped)
        .await;

    assert!(matches!(refused, Err(Error::InvalidTransition { .. })));
}
