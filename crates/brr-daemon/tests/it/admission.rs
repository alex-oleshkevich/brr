#![allow(clippy::unwrap_used)]

use crate::common;

use std::{sync::Arc, time::Duration};

use brr_core::{
    AdmissionMode, ClientId, ClientKind, Display, EndReason, Engine, Error, Host, IdleOverrides,
    InstanceId, InstanceState, Order, Page, SandboxMode, now_ms,
};
use brr_daemon::{
    App, CallClient, ops,
    services::{
        admission::{AdmitRequest, RETRY_AFTER_S},
        instances::LaunchRequest,
    },
};
use brr_store::event;
use serde_json::{Value, json};
use tokio::{sync::watch, task::JoinHandle, time};

use common::{
    KillOnDrop, add_profile, boot, chrome_app, ctx, row, scratch_app, triggers, wait_for,
};

const ONE_INSTANCE: (&str, &str) = ("BRR_LIMITS__MAX_INSTANCES", "1");
const ONE_SECOND_QUEUE: (&str, &str) = ("BRR_ADMISSION__QUEUE_TIMEOUT_S", "1");
const ONE_TAB: (&str, &str) = ("BRR_LIMITS__MAX_TABS_PER_INSTANCE", "1");
const TWO_TABS: (&str, &str) = ("BRR_LIMITS__MAX_TABS_PER_INSTANCE", "2");
const TWO_TABS_TOTAL: (&str, &str) = ("BRR_LIMITS__MAX_TABS_TOTAL", "2");
const TWO_INSTANCES: (&str, &str) = ("BRR_LIMITS__MAX_INSTANCES", "2");
const LONG_GRACE_S: u32 = 600;
// A headless Chrome with a page is well above this; the marked helpers alone are not.
const MIN_BROWSER_RSS_BYTES: u64 = 20 * 1024 * 1024;
const DEFAULT_PRIORITY: u8 = 5;
const HIGH_PRIORITY: u8 = 9;

fn request(priority: u8, mode: AdmissionMode) -> AdmitRequest {
    AdmitRequest {
        sandbox: SandboxMode::None,
        instance_id: InstanceId::new(),
        profile_name: "queued".to_owned(),
        engine: Engine::Chromium,
        display: Display::Headless,
        tags: Vec::new(),
        priority,
        mode,
        queued_at: now_ms(),
    }
}

fn client() -> CallClient {
    CallClient {
        client_id: ClientId::new(),
        kind: ClientKind::Cli,
        label: "admission".to_owned(),
        implicit: true,
    }
}

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    let mut context = ctx(app);
    context.client = client();

    ops::dispatch(context, ops::find(name).unwrap(), input).await
}

async fn queued_ids(app: &Arc<App>) -> Vec<InstanceId> {
    app.admission
        .queue()
        .await
        .into_iter()
        .map(|entry| entry.instance_id)
        .collect()
}

// Starts the request and returns once it is waiting in line.
async fn enqueue(
    app: &Arc<App>,
    request: AdmitRequest,
) -> (InstanceId, JoinHandle<Result<Option<Host>, Error>>) {
    let instance_id = request.instance_id;
    let waiting = queued_ids(app).await.len();
    let admit = {
        let app = Arc::clone(app);
        tokio::spawn(async move {
            app.admission
                .admit(&app, request, &mut watch::channel(None).1)
                .await
        })
    };
    wait_for("the request to queue", async || {
        queued_ids(app).await.len() == waiting + 1
    })
    .await;

    (instance_id, admit)
}

#[tokio::test]
async fn queue_priority_and_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[ONE_INSTANCE])).await.unwrap();

    let first = request(DEFAULT_PRIORITY, AdmissionMode::Queue);
    let first_id = first.instance_id;
    app.admission
        .admit(&app, first, &mut watch::channel(None).1)
        .await
        .unwrap();

    let (early, early_task) = enqueue(&app, request(DEFAULT_PRIORITY, AdmissionMode::Queue)).await;
    let (urgent, urgent_task) = enqueue(&app, request(HIGH_PRIORITY, AdmissionMode::Queue)).await;
    let (late, late_task) = enqueue(&app, request(DEFAULT_PRIORITY, AdmissionMode::Queue)).await;
    assert_eq!(queued_ids(&app).await, [urgent, early, late]);

    app.admission.finished(&app, &first_id).await;
    urgent_task.await.unwrap().unwrap();
    assert_eq!(queued_ids(&app).await, [early, late]);

    app.admission.finished(&app, &urgent).await;
    early_task.await.unwrap().unwrap();
    assert_eq!(queued_ids(&app).await, [late]);

    app.admission.finished(&app, &early).await;
    late_task.await.unwrap().unwrap();
    assert!(queued_ids(&app).await.is_empty());
}

#[tokio::test]
async fn queue_timeout_removes_the_waiter() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[ONE_INSTANCE, ONE_SECOND_QUEUE]))
        .await
        .unwrap();
    app.admission
        .admit(
            &app,
            request(DEFAULT_PRIORITY, AdmissionMode::Queue),
            &mut watch::channel(None).1,
        )
        .await
        .unwrap();

    let timed_out = app
        .admission
        .admit(
            &app,
            request(DEFAULT_PRIORITY, AdmissionMode::Queue),
            &mut watch::channel(None).1,
        )
        .await;

    assert_eq!(
        timed_out,
        Err(Error::CapExceeded {
            retry_after: Some(RETRY_AFTER_S),
            queue_pos: None
        })
    );
    assert!(queued_ids(&app).await.is_empty());
}

#[tokio::test]
async fn reject_mode_fails_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[ONE_INSTANCE])).await.unwrap();
    app.admission
        .admit(
            &app,
            request(DEFAULT_PRIORITY, AdmissionMode::Queue),
            &mut watch::channel(None).1,
        )
        .await
        .unwrap();

    let rejected = app
        .admission
        .admit(
            &app,
            request(DEFAULT_PRIORITY, AdmissionMode::Reject),
            &mut watch::channel(None).1,
        )
        .await;

    assert!(matches!(
        rejected,
        Err(Error::CapExceeded {
            retry_after: Some(RETRY_AFTER_S),
            ..
        })
    ));
    assert!(queued_ids(&app).await.is_empty());
}

#[tokio::test]
async fn limits_set() {
    let (_dir, app) = scratch_app().await;

    let refused = call(&app, "limits.set", json!({ "max_instances": 0 })).await;
    assert!(matches!(
        refused,
        Err(Error::InvalidArgument { field, .. }) if field == "max_instances"
    ));

    call(&app, "limits.set", json!({ "mode": "adaptive" }))
        .await
        .unwrap();
    let shown = call(&app, "limits.get", json!({})).await.unwrap();
    assert_eq!(shown["mode"], "adaptive");

    let config = std::fs::read_to_string(app.paths.config_file()).unwrap();
    assert!(config.contains("mode = \"adaptive\""), "{config}");

    let query = event::Query {
        since: None,
        kinds: vec!["limits.changed".to_owned()],
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: Order::Asc,
        page: Page::new(Some(10), None),
    };
    let (events, _) = app
        .store
        .run(move |c| event::list(c, &query))
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["keys"], json!(["limits.mode"]));
}

#[tokio::test]
async fn stop_cancels_queued() {
    let Some((_dir, app, build_id)) = chrome_app(&[ONE_INSTANCE]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, "first", false, IdleOverrides::default()).await;
    let waiting_profile =
        add_profile(&app, build_id, "second", false, IdleOverrides::default()).await;
    let launch = |profile: &brr_core::Profile| LaunchRequest {
        profile: profile.clone(),
        display: None,
        priority: DEFAULT_PRIORITY,
        admission: AdmissionMode::Queue,
        force: false,
    };

    let (running, _) = app.instances.launch(&app, launch(&profile)).await.unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    let queued_launch = {
        let app = Arc::clone(&app);
        let request = launch(&waiting_profile);
        tokio::spawn(async move { app.instances.launch(&app, request).await })
    };
    wait_for("the second launch to queue", async || {
        !queued_ids(&app).await.is_empty()
    })
    .await;
    let queued_id = queued_ids(&app).await[0];
    assert_eq!(row(&app, queued_id).await.state, InstanceState::Queued);

    let stopped = app
        .instances
        .stop(&app, &queued_id, EndReason::Stop)
        .await
        .unwrap();

    assert_eq!(stopped.state, InstanceState::Stopped);
    assert_eq!(stopped.end_reason, Some(EndReason::Cancel));
    assert!(matches!(
        queued_launch.await.unwrap(),
        Err(Error::CapExceeded { .. })
    ));
    assert!(queued_ids(&app).await.is_empty());
    assert_eq!(
        triggers(&app, queued_id).await.last().map(String::as_str),
        Some("cancel")
    );
}

#[tokio::test]
async fn evict_mode_reaps_an_idle_instance() {
    let Some((_dir, app, build_id)) = chrome_app(&[ONE_INSTANCE]).await else {
        return;
    };
    // instance_min = 0: the Instance idles and starts reaping as soon as it runs.
    let idler = IdleOverrides {
        instance_min: Some(0),
        grace_s: Some(LONG_GRACE_S),
        ..IdleOverrides::default()
    };
    let old = add_profile(&app, build_id, "old", false, idler).await;
    let new = add_profile(&app, build_id, "new", false, IdleOverrides::default()).await;
    let launch = |profile: &brr_core::Profile, admission| LaunchRequest {
        profile: profile.clone(),
        display: None,
        priority: DEFAULT_PRIORITY,
        admission,
        force: false,
    };

    let (old_instance, _) = app
        .instances
        .launch(&app, launch(&old, AdmissionMode::Queue))
        .await
        .unwrap();
    // The published view, not the row: the row turns Idle before the owner publishes the view,
    // and admission picks its victim from the views.
    wait_for("the old instance to idle", async || {
        app.instances.view(&old_instance.id).is_some_and(|view| {
            matches!(
                view.instance.state,
                InstanceState::Idle | InstanceState::Reaping
            )
        })
    })
    .await;

    let (fresh, _) = app
        .instances
        .launch(&app, launch(&new, AdmissionMode::Evict))
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![old_instance.id, fresh.id]);

    wait_for("the old instance to end", async || {
        row(&app, old_instance.id).await.state.is_final()
    })
    .await;
    assert_eq!(
        row(&app, old_instance.id).await.end_reason,
        Some(EndReason::Evicted)
    );
    app.instances
        .stop(&app, &fresh.id, EndReason::Stop)
        .await
        .unwrap();
}

// Reacting to the state change the moment it is announced must already find the Instance
// evictable: the owner publishes its view before the event.
#[tokio::test]
async fn evict_right_after_the_idle_event_succeeds() {
    let Some((_dir, app, build_id)) = chrome_app(&[ONE_INSTANCE]).await else {
        return;
    };
    let idler = IdleOverrides {
        instance_min: Some(0),
        grace_s: Some(LONG_GRACE_S),
        ..IdleOverrides::default()
    };
    let old = add_profile(&app, build_id, "old", false, idler).await;
    let new = add_profile(&app, build_id, "new", false, IdleOverrides::default()).await;
    let mut events = app.events.subscribe();

    let (old_instance, _) = app
        .instances
        .launch(
            &app,
            LaunchRequest {
                profile: old,
                display: None,
                priority: DEFAULT_PRIORITY,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap();
    let old_id = old_instance.id.to_string();
    loop {
        let published = events.recv().await.unwrap();
        let event = &published.event;
        if event.kind == "instance.state_changed"
            && event.subject.id == old_id
            && event.data["to"] == "idle"
        {
            break;
        }
    }

    let launched = time::timeout(
        Duration::from_secs(10),
        app.instances.launch(
            &app,
            LaunchRequest {
                profile: new,
                display: None,
                priority: DEFAULT_PRIORITY,
                admission: AdmissionMode::Evict,
                force: false,
            },
        ),
    )
    .await
    .expect("the eviction was not found promptly")
    .unwrap();
    let _guard = KillOnDrop::new(&app, vec![old_instance.id, launched.0.id]);

    wait_for("the old instance to end", async || {
        row(&app, old_instance.id).await.state.is_final()
    })
    .await;
    assert_eq!(
        row(&app, old_instance.id).await.end_reason,
        Some(EndReason::Evicted)
    );
}

#[tokio::test]
async fn tab_cap_and_rss() {
    let Some((_dir, app, build_id)) = chrome_app(&[ONE_TAB]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, "tabs", false, IdleOverrides::default()).await;
    let (running, _) = app
        .instances
        .launch(
            &app,
            LaunchRequest {
                profile,
                display: None,
                priority: DEFAULT_PRIORITY,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    let refused = call(
        &app,
        "tab.open",
        json!({ "instance_id": running.id.to_string() }),
    )
    .await;
    assert_eq!(
        refused.unwrap_err(),
        Error::CapExceeded {
            retry_after: Some(RETRY_AFTER_S),
            queue_pos: None
        }
    );

    let shown = call(
        &app,
        "instance.get",
        json!({ "instance_id": running.id.to_string() }),
    )
    .await
    .unwrap();
    assert!(shown["rss_bytes"].as_u64().unwrap() > MIN_BROWSER_RSS_BYTES);
}

// Two evict-mode launches at once need two victims, not one victim twice.
#[tokio::test]
async fn concurrent_evictions_pick_different_victims() {
    let Some((_dir, app, build_id)) = chrome_app(&[TWO_INSTANCES]).await else {
        return;
    };
    let idler = IdleOverrides {
        instance_min: Some(0),
        grace_s: Some(LONG_GRACE_S),
        ..IdleOverrides::default()
    };
    let launch = |profile: &brr_core::Profile, admission| LaunchRequest {
        profile: profile.clone(),
        display: None,
        priority: DEFAULT_PRIORITY,
        admission,
        force: false,
    };

    let mut old = Vec::new();
    for name in ["old-a", "old-b"] {
        let profile = add_profile(&app, build_id, name, false, idler.clone()).await;
        let (instance, _) = app
            .instances
            .launch(&app, launch(&profile, AdmissionMode::Queue))
            .await
            .unwrap();
        old.push(instance.id);
    }
    wait_for("both old instances to start reaping", async || {
        for id in &old {
            if row(&app, *id).await.state != InstanceState::Reaping {
                return false;
            }
        }
        true
    })
    .await;

    let first = add_profile(&app, build_id, "new-a", false, IdleOverrides::default()).await;
    let second = add_profile(&app, build_id, "new-b", false, IdleOverrides::default()).await;
    let (a, b) = tokio::join!(
        app.instances
            .launch(&app, launch(&first, AdmissionMode::Evict)),
        app.instances
            .launch(&app, launch(&second, AdmissionMode::Evict)),
    );
    let (a, b) = (a.unwrap().0, b.unwrap().0);
    let _guard = KillOnDrop::new(&app, vec![a.id, b.id]);

    for id in &old {
        assert_eq!(row(&app, *id).await.end_reason, Some(EndReason::Evicted));
    }
    for id in [a.id, b.id] {
        app.instances
            .stop(&app, &id, EndReason::Stop)
            .await
            .unwrap();
    }
}

async fn concurrent_opens(app: &Arc<App>, instance_id: InstanceId) -> Vec<Result<Value, Error>> {
    let opens = (0..3).map(|_| {
        call(
            app,
            "tab.open",
            json!({ "instance_id": instance_id.to_string() }),
        )
    });

    futures::future::join_all(opens).await
}

// One Tab is open and the limit is two: of three simultaneous opens exactly one gets in.
#[tokio::test]
async fn concurrent_tab_opens_respect_the_instance_limit() {
    let Some((_dir, app, build_id)) = chrome_app(&[TWO_TABS]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, "tabs", false, IdleOverrides::default()).await;
    let (running, _) = app
        .instances
        .launch(&app, plain_launch(profile))
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    let results = concurrent_opens(&app, running.id).await;

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .all(|error| {
                matches!(
                    error,
                    Error::CapExceeded {
                        retry_after: Some(RETRY_AFTER_S),
                        ..
                    }
                )
            })
    );
}

#[tokio::test]
async fn concurrent_tab_opens_respect_the_total_limit() {
    let Some((_dir, app, build_id)) = chrome_app(&[TWO_TABS_TOTAL]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, "total", false, IdleOverrides::default()).await;
    let (running, _) = app
        .instances
        .launch(&app, plain_launch(profile))
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    let results = concurrent_opens(&app, running.id).await;

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
}

fn plain_launch(profile: brr_core::Profile) -> LaunchRequest {
    LaunchRequest {
        profile,
        display: None,
        priority: DEFAULT_PRIORITY,
        admission: AdmissionMode::Queue,
        force: false,
    }
}
