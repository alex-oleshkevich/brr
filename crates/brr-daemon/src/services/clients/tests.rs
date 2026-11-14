use std::{ffi::OsString, fs, sync::Arc, time::Duration};

use brr_core::{ClientKind, DisplayEnv, Error, InstanceId, Order, Page, Paths, TabId, settings};
use brr_store::{client, event};
use tempfile::TempDir;
use tokio::{task, time};

use crate::{App, Boot, CallClient};

const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const REST_IDLE: Duration = Duration::from_secs(10 * 60);
const CONFIG_WITH_FILE_VAULT: &str = "[vault]\nkey_source = \"file\"\n";

async fn scratch_app() -> (TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths {
        config_dir: dir.path().join("config"),
        data_dir: dir.path().join("data"),
        state_dir: dir.path().join("state"),
        runtime_dir: dir.path().join("run").join("brr"),
    };
    fs::create_dir_all(&paths.config_dir).unwrap();
    fs::write(paths.config_file(), CONFIG_WITH_FILE_VAULT).unwrap();
    let (settings, sources) = settings::load(&paths.config_file(), &[]).unwrap();

    let app = App::start(Boot {
        paths,
        settings,
        sources,
        env: Vec::new(),
        display_env: DisplayEnv::default(),
        path_env: OsString::new(),
        cft_base_url: String::new(),
        version: "0.0.0-test",
    })
    .await
    .unwrap();

    (dir, app)
}

async fn kinds(app: &App) -> Vec<String> {
    let query = event::Query {
        since: None,
        kinds: Vec::new(),
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: Order::Asc,
        page: Page::new(None, None),
    };
    let (events, _) = app
        .store
        .run(move |c| event::list(c, &query))
        .await
        .unwrap();

    events.into_iter().map(|e| e.kind).collect()
}

async fn settled(mut done: impl AsyncFnMut() -> bool) {
    time::timeout(SETTLE_TIMEOUT, async {
        while !done().await {
            task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn ended_reason(app: &App, call: &CallClient) -> Option<String> {
    let id = call.client_id.clone();
    let stored = app.store.run(move |c| client::get(c, &id)).await.unwrap();
    stored.ended_at?;

    let query = event::Query {
        since: None,
        kinds: vec!["client.ended".to_string()],
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: Order::Desc,
        page: Page::new(Some(1), None),
    };
    let (ended, _) = app
        .store
        .run(move |c| event::list(c, &query))
        .await
        .unwrap();

    ended.first()?.data["reason"].as_str().map(str::to_string)
}

#[tokio::test]
async fn rest_header_reuses_the_client_and_rejects_bad_ids() {
    let (_dir, app) = scratch_app().await;

    let first = app.clients.resolve_rest(&app, "my-script").await.unwrap();
    let second = app.clients.resolve_rest(&app, "my-script").await.unwrap();

    assert_eq!(first.client_id, second.client_id);
    assert_eq!(first.kind, ClientKind::Rest);
    assert_eq!(first.label, "my-script");
    let created = kinds(&app)
        .await
        .iter()
        .filter(|k| *k == "client.created")
        .count();
    assert_eq!(created, 1);

    let bad = app.clients.resolve_rest(&app, "bad id with spaces").await;
    assert!(matches!(bad, Err(Error::InvalidArgument { field, .. }) if field == "client_id"));
}

#[tokio::test]
async fn last_hold_dropping_ends_an_mcp_client_and_releases_its_leases() {
    let (_dir, app) = scratch_app().await;
    let created = app
        .clients
        .create(&app, ClientKind::Mcp, "agent".to_string())
        .await
        .unwrap();
    let call = app.clients.resolve(&app, &created.id).await.unwrap();
    let instance_id = InstanceId::new();
    app.leases
        .acquire_tab(&app, &call, &instance_id, &TabId::new())
        .await
        .unwrap();

    let hold = app.clients.hold(&app, &created.id).await.unwrap();
    drop(hold);

    settled(async || ended_reason(&app, &call).await.is_some()).await;
    assert!(app.leases.list(None, None).await.is_empty());
    assert_eq!(
        ended_reason(&app, &call).await.as_deref(),
        Some("disconnected")
    );
}

#[tokio::test(start_paused = true)]
async fn rest_client_ends_after_the_idle_limit() {
    let (_dir, app) = scratch_app().await;
    let call = app.clients.resolve_rest(&app, "idle-script").await.unwrap();
    // Lets the expiry task register its timer before the clock moves.
    task::yield_now().await;
    task::yield_now().await;

    time::advance(REST_IDLE + Duration::from_secs(1)).await;

    settled(async || ended_reason(&app, &call).await.is_some()).await;
    assert_eq!(ended_reason(&app, &call).await.as_deref(), Some("idle"));
}

#[tokio::test(start_paused = true)]
async fn idle_timer_waits_for_calls_in_flight() {
    let (_dir, app) = scratch_app().await;
    let call = app.clients.resolve_rest(&app, "busy-script").await.unwrap();
    // Lets the expiry task register its timer before the call starts.
    task::yield_now().await;
    task::yield_now().await;
    let guard = app.clients.in_call(&app, &call.client_id).await;
    task::yield_now().await;
    task::yield_now().await;

    time::advance(REST_IDLE * 2).await;
    task::yield_now().await;
    assert!(app.clients.is_active(&call.client_id).await);

    drop(guard);
    settled(async || app.clients.busy(&call.client_id).await == Some(false)).await;
    // Lets the expiry task re-arm its timer before the clock moves.
    task::yield_now().await;
    task::yield_now().await;
    time::advance(REST_IDLE + Duration::from_secs(1)).await;

    settled(async || ended_reason(&app, &call).await.is_some()).await;
    assert_eq!(ended_reason(&app, &call).await.as_deref(), Some("idle"));
}

#[tokio::test]
async fn rest_header_does_not_revive_another_kind() {
    let (_dir, app) = scratch_app().await;
    let created = app
        .clients
        .create(&app, ClientKind::Mcp, "agent".to_string())
        .await
        .unwrap();
    app.clients
        .end(&app, &created.id, brr_core::ClientEndReason::Disconnected)
        .await
        .unwrap();

    let revived = app
        .clients
        .resolve_rest(&app, &created.id.to_string())
        .await;

    assert!(matches!(revived, Err(Error::NotFound { .. })));
    assert!(!app.clients.is_active(&created.id).await);
}
