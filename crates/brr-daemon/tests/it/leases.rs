#![allow(clippy::unwrap_used)]

use crate::common;

use std::{sync::Arc, time::Duration};

use brr_core::{
    Client, ClientId, ClientKind, Error, InstanceId, InstanceState, Lease, ReleaseReason, TabId,
    TabState, now_ms,
};
use brr_daemon::{App, CallClient, ops};
use brr_store::{Store, client, instance, lease, tab};
use serde_json::{Value, json};
use tokio::time;

use common::{boot, ctx, instance_row, paths, scratch_app, settled, tab_row};

const EXCLUSIVE_WAIT_ENV: (&str, &str) = ("BRR_LEASE__EXCLUSIVE_WAIT_S", "1");
const HALF_WAIT: Duration = Duration::from_millis(500);

fn call_client(label: &str) -> CallClient {
    CallClient {
        client_id: ClientId::new(),
        kind: ClientKind::Mcp,
        label: label.to_string(),
        implicit: true,
    }
}

async fn seed_tab(
    app: &App,
    instance_state: InstanceState,
    tab_state: TabState,
) -> (InstanceId, TabId) {
    let instance = instance_row(instance_state, None);
    let tab = tab_row(instance.id, tab_state);
    let ids = (instance.id, tab.id);
    app.store
        .run(move |c| {
            instance::insert(c, &instance)?;
            tab::insert(c, &tab)
        })
        .await
        .unwrap();

    ids
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

#[tokio::test]
async fn lease_held_and_read_only() {
    let (_dir, app) = scratch_app().await;
    let (a, b) = (call_client("a"), call_client("b"));
    let (instance_id, tab_id) = (InstanceId::new(), TabId::new());
    app.leases
        .acquire_tab(&app, &a, &instance_id, &tab_id)
        .await
        .unwrap();

    let blocked = app
        .leases
        .guard(&app, &b, &instance_id, Some(&tab_id), false)
        .await;
    assert_eq!(
        blocked,
        Err(Error::LeaseHeld {
            client_id: a.client_id.clone()
        })
    );

    app.leases
        .guard(&app, &b, &instance_id, Some(&tab_id), true)
        .await
        .unwrap();
    assert!(app.leases.list(None, Some(&b.client_id)).await.is_empty());

    let other_tab = TabId::new();
    app.leases
        .guard(&app, &b, &instance_id, Some(&other_tab), false)
        .await
        .unwrap();
    assert_eq!(
        app.leases.tab_lease(&other_tab).await.unwrap().client_id,
        b.client_id
    );

    let released = app
        .leases
        .release(
            &app,
            &b,
            &app.leases.tab_lease(&tab_id).await.unwrap().lease_id,
        )
        .await;
    assert_eq!(
        released,
        Err(Error::LeaseHeld {
            client_id: a.client_id.clone()
        })
    );
}

#[tokio::test(start_paused = true)]
async fn exclusive_waits_then_fails() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[EXCLUSIVE_WAIT_ENV]))
        .await
        .unwrap();
    let (a, b) = (call_client("a"), call_client("b"));
    let instance_id = InstanceId::new();

    let tab_lease = app
        .leases
        .acquire_tab(&app, &b, &instance_id, &TabId::new())
        .await
        .unwrap();
    let waiting = {
        let (app, a) = (Arc::clone(&app), a.clone());
        tokio::spawn(async move { app.leases.acquire_exclusive(&app, &a, &instance_id).await })
    };
    time::sleep(HALF_WAIT).await;
    app.leases.release(&app, &b, &tab_lease.id).await.unwrap();

    let exclusive = waiting.await.unwrap().unwrap();
    assert!(exclusive.exclusive);
    assert_eq!(exclusive.client_id, a.client_id);

    let blocked = app
        .leases
        .acquire_tab(&app, &b, &instance_id, &TabId::new())
        .await;
    assert_eq!(
        blocked,
        Err(Error::LeaseHeld {
            client_id: a.client_id.clone()
        })
    );
    let blocked = app.leases.guard(&app, &b, &instance_id, None, false).await;
    assert_eq!(
        blocked,
        Err(Error::LeaseHeld {
            client_id: a.client_id.clone()
        })
    );

    let other_instance = InstanceId::new();
    app.leases
        .acquire_tab(&app, &b, &other_instance, &TabId::new())
        .await
        .unwrap();
    let timed_out = app
        .leases
        .acquire_exclusive(&app, &a, &other_instance)
        .await;
    assert_eq!(
        timed_out,
        Err(Error::LeaseHeld {
            client_id: b.client_id.clone()
        })
    );
}

#[tokio::test]
async fn implicit_drop_releases() {
    let (_dir, app) = scratch_app().await;
    let implicit = app.clients.implicit(&app, ClientKind::Rest);
    let mut events = app.events.subscribe();
    app.leases
        .acquire_tab(&app, &implicit.client, &InstanceId::new(), &TabId::new())
        .await
        .unwrap();
    assert_eq!(app.leases.list(None, None).await.len(), 1);

    drop(implicit);

    settled(async || app.leases.list(None, None).await.is_empty()).await;
    let mut reason = None;
    while let Ok(published) = events.try_recv() {
        if published.event.kind == "lease.released" {
            reason = published.event.data["reason"].as_str().map(str::to_string);
        }
    }
    assert_eq!(reason.as_deref(), Some("client_ended"));
}

#[tokio::test]
async fn reconcile_on_start() {
    let dir = tempfile::tempdir().unwrap();
    let db_file = paths(dir.path()).db_file();
    let (stale_client, stale_lease) = (
        Client {
            id: ClientId::new(),
            kind: ClientKind::Rest,
            label: "stale".to_string(),
            current_tab_id: None,
            created_at: now_ms(),
            ended_at: None,
        },
        Lease {
            id: brr_core::LeaseId::new(),
            client_id: ClientId::new(),
            client_kind: ClientKind::Rest,
            client_label: "stale".to_string(),
            instance_id: InstanceId::new(),
            tab_id: Some(TabId::new()),
            exclusive: false,
            acquired_at: now_ms(),
            released_at: None,
            release_reason: None,
        },
    );
    {
        let store = Store::open(&db_file).unwrap();
        let (c, l) = (stale_client.clone(), stale_lease.clone());
        store
            .run(move |conn| {
                client::upsert(conn, &c)?;
                lease::insert(conn, &l)
            })
            .await
            .unwrap();
    }

    let app = App::start(boot(dir.path(), &[])).await.unwrap();

    let (lease_id, client_id) = (stale_lease.id, stale_client.id.clone());
    let (ended, active, reason) = app
        .store
        .run(move |c| {
            let reason: String = c
                .query_row(
                    "SELECT release_reason FROM lease WHERE id = ?1",
                    [lease_id.to_string()],
                    |row| row.get(0),
                )
                .map_err(|e| Error::Internal {
                    message: e.to_string(),
                })?;

            Ok((client::get(c, &client_id)?, lease::active(c)?, reason))
        })
        .await
        .unwrap();
    assert!(ended.ended_at.is_some());
    assert!(active.is_empty());
    assert_eq!(reason, "daemon_restart");

    let query = brr_store::event::Query {
        since: None,
        kinds: vec!["client.ended".to_string()],
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: brr_core::Order::Asc,
        page: brr_core::Page::new(None, None),
    };
    let (logged, _) = app
        .store
        .run(move |c| brr_store::event::list(c, &query))
        .await
        .unwrap();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].data["client_id"], stale_client.id.to_string());
    assert_eq!(logged[0].data["reason"], "daemon_restart");
}

#[tokio::test]
async fn ended_client_cannot_acquire() {
    let (_dir, app) = scratch_app().await;
    let created = app
        .clients
        .create(&app, ClientKind::Rest, "script".to_string())
        .await
        .unwrap();
    let call = app.clients.resolve(&app, &created.id).await.unwrap();
    app.clients
        .end(&app, &created.id, brr_core::ClientEndReason::Ended)
        .await
        .unwrap();

    let result = app
        .leases
        .acquire_tab(&app, &call, &InstanceId::new(), &TabId::new())
        .await;

    assert!(matches!(result, Err(Error::NotFound { .. })));
    assert!(app.leases.list(None, None).await.is_empty());
}

#[tokio::test]
async fn lease_ops_validate_and_page() {
    let (_dir, app) = scratch_app().await;
    let a = call_client("a");
    let (instance_id, tab_id) = seed_tab(&app, InstanceState::Running, TabState::Ready).await;
    let (_, closed_tab) = seed_tab(&app, InstanceState::Running, TabState::Closed).await;
    let (stopped_instance, stopped_tab) =
        seed_tab(&app, InstanceState::Stopped, TabState::Ready).await;

    let acquired = call(
        &app,
        &a,
        "lease.acquire",
        json!({"instance_id": instance_id, "tab_id": tab_id}),
    )
    .await
    .unwrap();
    assert_eq!(acquired["client_id"], a.client_id.to_string());
    assert_eq!(acquired["tab_id"], tab_id.to_string());

    let closed = call(
        &app,
        &a,
        "lease.acquire",
        json!({"instance_id": instance_id, "tab_id": closed_tab}),
    )
    .await;
    assert!(matches!(closed, Err(Error::NotFound { .. })));
    let stopped = call(
        &app,
        &a,
        "lease.acquire",
        json!({"instance_id": stopped_instance, "tab_id": stopped_tab}),
    )
    .await;
    assert!(matches!(stopped, Err(Error::InvalidTransition { .. })));
    let neither = call(
        &app,
        &a,
        "lease.acquire",
        json!({"instance_id": instance_id}),
    )
    .await;
    assert!(matches!(neither, Err(Error::InvalidArgument { field, .. }) if field == "tab_id"));

    let exclusive = call(
        &app,
        &a,
        "lease.acquire",
        json!({"instance_id": instance_id, "exclusive": true}),
    )
    .await
    .unwrap();
    assert_eq!(exclusive["exclusive"], true);

    let first = call(&app, &a, "lease.list", json!({"limit": 1}))
        .await
        .unwrap();
    // Same-millisecond Leases tie-break by id, so the expected order uses the list's own key.
    let mut expected = [acquired.clone(), exclusive.clone()];
    expected.sort_by_key(|l| {
        (
            l["acquired_at"].as_i64(),
            l["id"].as_str().map(str::to_string),
        )
    });
    assert_eq!(first["leases"][0]["id"], expected[0]["id"]);
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = call(
        &app,
        &a,
        "lease.list",
        json!({"limit": 1, "cursor": cursor}),
    )
    .await
    .unwrap();
    assert_eq!(second["leases"][0]["id"], expected[1]["id"]);
    assert_eq!(second["next_cursor"], Value::Null);

    let released = call(
        &app,
        &a,
        "lease.release",
        json!({"lease_id": acquired["id"]}),
    )
    .await
    .unwrap();
    assert_eq!(released["release_reason"], json!(ReleaseReason::Released));
    assert_eq!(app.leases.count(&instance_id).await, 1);
}

#[tokio::test]
async fn client_ops_create_and_end() {
    let (_dir, app) = scratch_app().await;
    let caller = call_client("caller");

    let created = call(&app, &caller, "client.create", json!({"label": "scripted"}))
        .await
        .unwrap();
    assert_eq!(created["kind"], "mcp");
    assert_eq!(created["label"], "scripted");
    let client_id: ClientId = serde_json::from_value(created["id"].clone()).unwrap();
    let held = call_client("held");
    let lease_client = app.clients.resolve(&app, &client_id).await.unwrap();
    app.leases
        .acquire_tab(&app, &lease_client, &InstanceId::new(), &TabId::new())
        .await
        .unwrap();
    app.leases
        .acquire_tab(&app, &held, &InstanceId::new(), &TabId::new())
        .await
        .unwrap();

    let ended = call(&app, &caller, "client.end", json!({"client_id": client_id}))
        .await
        .unwrap();
    assert!(ended["ended_at"].is_i64());
    assert!(app.leases.list(None, Some(&client_id)).await.is_empty());
    assert_eq!(app.leases.list(None, None).await.len(), 1);

    let again = call(&app, &caller, "client.end", json!({"client_id": client_id}))
        .await
        .unwrap();
    assert_eq!(again["ended_at"], ended["ended_at"]);
    let too_long = call(
        &app,
        &caller,
        "client.create",
        json!({"label": "x".repeat(65)}),
    )
    .await;
    assert!(matches!(too_long, Err(Error::InvalidArgument { field, .. }) if field == "label"));
}

// The store connection is held busy, so the release is cancelled with its write still queued.
#[tokio::test]
async fn cancelled_release_still_finishes_and_announces() {
    let (_dir, app) = scratch_app().await;
    let client = call_client("a");
    let lease = app
        .leases
        .acquire_tab(&app, &client, &InstanceId::new(), &TabId::new())
        .await
        .unwrap();
    let mut events = app.events.subscribe();

    let (unblock, blocked) = std::sync::mpsc::channel::<()>();
    let (started_tx, started) = tokio::sync::oneshot::channel();
    let busy = {
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            app.store
                .run(move |_| {
                    let _ = started_tx.send(());
                    let _ = blocked.recv();

                    Ok(())
                })
                .await
        })
    };
    started.await.unwrap();

    let release = {
        let (app, client) = (Arc::clone(&app), client.clone());
        tokio::spawn(async move { app.leases.release(&app, &client, &lease.id).await })
    };
    tokio::task::yield_now().await;
    release.abort();
    let _ = release.await;
    unblock.send(()).unwrap();
    busy.await.unwrap().unwrap();

    settled(async || {
        app.store
            .run(|c| lease::active(c))
            .await
            .unwrap()
            .is_empty()
    })
    .await;
    let mut announced = false;
    time::timeout(Duration::from_secs(5), async {
        while !announced {
            let published = events.recv().await.unwrap();
            announced = published.event.kind == "lease.released"
                && published.event.data["lease_id"] == lease.id.to_string();
        }
    })
    .await
    .unwrap();
    assert!(app.leases.list(None, None).await.is_empty());
}
