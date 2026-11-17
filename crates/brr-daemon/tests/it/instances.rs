#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{
    fs,
    os::unix::process::ExitStatusExt,
    process::{Child, Command},
    sync::Arc,
    time::Duration,
};

use brr_core::{
    AdmissionMode, BuildChoice, BuildId, ConflictReason, Display, DisplayEnv, EndReason, Engine,
    Error, Instance, InstanceId, InstanceState, Profile, ProfileId, ProfileKind, ProfileSettings,
    TabCloseReason, TabId, TabState, TargetId, now_ms,
};
use brr_daemon::{
    App, CallClient,
    services::instances::{AttachSource, LaunchRequest},
};
use brr_engine::procs::parse_environ;
use brr_engine::{Driver, LaunchSpec};
use brr_store::{Store, instance, profile, tab};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use tokio::time;

use common::{
    KillOnDrop, boot, chrome, chrome_app, events_of, instance_row, paths, profile_row,
    register_build, row, scratch_app, triggers, wait_for,
};

const POLL_TIMEOUT: Duration = Duration::from_secs(30);
const DECOY_EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const ONE_INSTANCE: (&str, &str) = ("BRR_LIMITS__MAX_INSTANCES", "1");
const START_TIMEOUT: (&str, &str) = ("BRR_INSTANCE__START_TIMEOUT_S", "4");
const NO_RENDERER: &str = "--renderer-cmd-prefix=/bin/false";
const STOP_AGAIN_TIMEOUT: Duration = Duration::from_secs(5);
const NO_BROWSER: &str = "/nonexistent/chrome";
const STATE_CHANGED: &str = "instance.state_changed";
const LEASE_RELEASED: &str = "lease.released";
const AUTO_RESTART_ENV: (&str, &str) = ("BRR_INSTANCE__AUTO_RESTART", "true");

fn decoy(instance_id: InstanceId) -> Child {
    Command::new("sleep")
        .arg("300")
        .env("BRR_INSTANCE_ID", instance_id.to_string())
        .spawn()
        .unwrap()
}

async fn wait_final(app: &Arc<App>, instance_id: InstanceId) -> Instance {
    wait_for("a final instance state", async || {
        row(app, instance_id).await.state.is_final()
    })
    .await;

    row(app, instance_id).await
}

fn marked_pids(instance_id: InstanceId) -> Vec<u32> {
    fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse().ok()?;
            let environ = fs::read(entry.path().join("environ")).ok()?;

            (parse_environ(&environ) == Some(instance_id)).then_some(pid)
        })
        .collect()
}

async fn add_profile(app: &Arc<App>, build_id: BuildId, kind: ProfileKind, name: &str) -> Profile {
    let id = ProfileId::new();
    let data_dir =
        (kind == ProfileKind::Persistent).then(|| app.paths.profiles_dir().join(id.to_string()));
    let now = now_ms();
    let created = Profile {
        id,
        name: name.to_owned(),
        kind,
        template_id: None,
        template_name: None,
        settings: ProfileSettings {
            build: BuildChoice::BuildId(build_id),
            display: Display::Headless,
            ..ProfileSettings::default()
        },
        pinned: false,
        notes: String::new(),
        tags: Vec::new(),
        data_dir,
        disk_quota_bytes: None,
        backup_schedule: None,
        backup_keep: 0,
        created_at: now,
        updated_at: now,
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    };
    let row = created.clone();
    app.store
        .run(move |c| profile::insert(c, &row))
        .await
        .unwrap();

    created
}

fn request(profile: &Profile) -> LaunchRequest {
    LaunchRequest {
        profile: profile.clone(),
        display: None,
        priority: 5,
        admission: AdmissionMode::Queue,
        force: false,
    }
}

#[tokio::test]
async fn reconcile_kills_only_own_markers() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    fs::create_dir_all(&paths.data_dir).unwrap();

    let ephemeral_data = paths.ephemeral_dir().join("stale-instance");
    let leftover = paths.ephemeral_dir().join("leftover");
    for data in [&ephemeral_data, &leftover] {
        fs::create_dir_all(data).unwrap();
        fs::write(data.join("cookies"), "x").unwrap();
    }

    let stale = instance_row(InstanceState::Running, Some(ephemeral_data.clone()));
    let stale_id = stale.id;
    let open_tab = brr_core::Tab {
        id: TabId::new(),
        instance_id: stale_id,
        target_id: TargetId("target".to_owned()),
        state: TabState::Opening,
        url: "about:blank".to_owned(),
        title: String::new(),
        opened_by_brr: true,
        pinned: false,
        last_activity_at: 0,
        created_at: 0,
        closed_at: None,
        lease: None,
    };
    let store = Store::open(&paths.db_file()).unwrap();
    store
        .run(move |c| {
            instance::insert(c, &stale)?;
            tab::insert(c, &open_tab)
        })
        .await
        .unwrap();
    drop(store);

    let mut own = decoy(stale_id);
    let mut foreign = decoy(InstanceId::new());

    let app = App::start(boot(dir.path(), &[])).await.unwrap();

    let own_status = time::timeout(DECOY_EXIT_TIMEOUT, async { own.wait().unwrap() }).await;
    let survived = foreign.try_wait().unwrap();
    foreign.kill().unwrap();
    foreign.wait().unwrap();
    assert_eq!(own_status.unwrap().signal(), Some(9));
    assert_eq!(survived, None);

    let reconciled = row(&app, stale_id).await;
    assert_eq!(reconciled.state, InstanceState::Stopped);
    assert_eq!(reconciled.end_reason, Some(EndReason::DaemonRestart));
    assert!(reconciled.ended_at.is_some());
    assert!(!ephemeral_data.exists());
    assert!(!leftover.exists());

    let changes = events_of(&app, STATE_CHANGED, stale_id).await;
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["trigger"], "reconcile");
    assert_eq!(changes[0]["reason"], "daemon_restart");

    let closed = events_of(&app, "tab.closed", stale_id).await;
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0]["reason"], "daemon_restart");
}

// A browser the daemon lost: Chromium rewrites its main process environ, so only helpers such
// as the crashpad handler still carry the marker.
#[tokio::test]
async fn reconcile_kills_marked_browser_processes() {
    let Some(chrome) = chrome() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    fs::create_dir_all(&paths.data_dir).unwrap();
    let orphan = instance_row(InstanceState::Running, None);
    let orphan_id = orphan.id;
    let store = Store::open(&paths.db_file()).unwrap();
    store
        .run(move |c| instance::insert(c, &orphan))
        .await
        .unwrap();
    drop(store);

    let launched = Driver::launch(LaunchSpec {
        instance_id: orphan_id,
        engine: Engine::Chromium,
        executable: chrome,
        user_data_dir: dir.path().join("browser-profile"),
        log_file: dir.path().join("browser.log"),
        display: Display::Headless,
        display_env: DisplayEnv::default(),
        settings: ProfileSettings::default(),
        start_timeout: POLL_TIMEOUT,
        extensions: Vec::new(),
        proxy_server: None,
        download_dir: dir.path().join("downloads"),
        console_buffer: 1000,
        network_buffer: 1000,
        camoufox: None,
        sandbox: None,
    })
    .await
    .unwrap();
    assert!(!marked_pids(orphan_id).is_empty());

    let app = App::start(boot(dir.path(), &[])).await.unwrap();
    launched.driver.close().await.unwrap();

    assert!(marked_pids(orphan_id).is_empty());
    assert_eq!(row(&app, orphan_id).await.state, InstanceState::Stopped);
}

// The parent runs the same binary as the marked child: it is the browser's main process.
#[tokio::test]
async fn reconcile_kills_same_binary_parent() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    fs::create_dir_all(&paths.data_dir).unwrap();
    let stale = instance_row(InstanceState::Running, None);
    let stale_id = stale.id;
    let store = Store::open(&paths.db_file()).unwrap();
    store
        .run(move |c| instance::insert(c, &stale))
        .await
        .unwrap();
    drop(store);

    let script = format!("BRR_INSTANCE_ID={stale_id} bash -c 'sleep 300; true'; true");
    let mut parent = Command::new("bash").args(["-c", &script]).spawn().unwrap();
    wait_for("the marked child", async || {
        !marked_pids(stale_id).is_empty()
    })
    .await;

    App::start(boot(dir.path(), &[])).await.unwrap();

    let status = time::timeout(DECOY_EXIT_TIMEOUT, async { parent.wait().unwrap() }).await;
    assert_eq!(status.unwrap().signal(), Some(9));
    assert!(marked_pids(stale_id).is_empty());
}

#[tokio::test]
async fn persistent_lock() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Persistent, "locked").await;

    let (running, first_tab) = app.instances.launch(&app, request(&profile)).await.unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    assert_eq!(running.state, InstanceState::Running);
    assert_eq!(first_tab.state, TabState::Ready);
    assert_eq!(first_tab.instance_id, running.id);
    assert_eq!(app.instances.view(&running.id).unwrap().tabs.len(), 1);

    let second = app.instances.launch(&app, request(&profile)).await.err();
    assert_eq!(
        second,
        Some(Error::Conflict {
            reason: ConflictReason::ProfileLocked,
            detail: None
        })
    );

    app.instances
        .stop(&app, &running.id, EndReason::Stop)
        .await
        .unwrap();
}

#[tokio::test]
async fn stop_then_invalid() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let (running, _) = app.instances.launch(&app, request(&profile)).await.unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    let data_dir = running.data_dir.clone().unwrap();
    assert!(data_dir.exists());

    let stopped = app
        .instances
        .stop(&app, &running.id, EndReason::Stop)
        .await
        .unwrap();

    assert_eq!(stopped.state, InstanceState::Stopped);
    assert_eq!(stopped.end_reason, Some(EndReason::Stop));
    assert_eq!(
        triggers(&app, running.id).await,
        ["launch", "admitted", "ready", "stop", "exited"]
    );
    assert!(marked_pids(running.id).is_empty());
    assert!(!data_dir.exists());

    let again = app.instances.stop(&app, &running.id, EndReason::Stop).await;
    assert_eq!(
        again,
        Err(Error::InvalidTransition {
            from: "stopped".to_owned(),
            to: "stopping".to_owned()
        })
    );
}

#[tokio::test]
async fn concurrent_stops_both_wait_for_final() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let (running, _) = app.instances.launch(&app, request(&profile)).await.unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    let (first, second) = tokio::join!(
        app.instances.stop(&app, &running.id, EndReason::Stop),
        app.instances.stop(&app, &running.id, EndReason::Stop)
    );

    assert_eq!(first.unwrap().state, InstanceState::Stopped);
    assert_eq!(second.unwrap().state, InstanceState::Stopped);
}

#[tokio::test]
async fn last_tab_closed() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let (running, only_tab) = app.instances.launch(&app, request(&profile)).await.unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    let closed = app
        .instances
        .close_tab(&running.id, &only_tab.id, TabCloseReason::Client)
        .await
        .unwrap();

    assert_eq!(closed.state, TabState::Closed);
    let ended = wait_final(&app, running.id).await;
    assert_eq!(ended.state, InstanceState::Stopped);
    assert_eq!(ended.end_reason, Some(EndReason::LastTabClosed));
    assert_eq!(
        triggers(&app, running.id).await,
        ["launch", "admitted", "ready", "last_tab_closed", "exited"]
    );
}

#[tokio::test]
async fn tabs_open_navigate_close() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let (running, first_tab) = app.instances.launch(&app, request(&profile)).await.unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    let second = app
        .instances
        .open_tab(
            &running.id,
            Some("data:text/html,<title>two</title>".to_owned()),
        )
        .await
        .unwrap();
    assert_ne!(second.state, TabState::Opening);
    app.instances
        .activate_tab(&running.id, &first_tab.id)
        .await
        .unwrap();
    app.instances
        .navigate(
            &running.id,
            &second.id,
            "data:text/html,<title>three</title>".to_owned(),
            brr_core::WaitUntil::Load,
            POLL_TIMEOUT,
        )
        .await
        .unwrap();
    wait_for("the new title", async || {
        let view = app.instances.view(&running.id).unwrap();
        view.tabs
            .iter()
            .any(|t| t.id == second.id && t.title == "three")
    })
    .await;

    app.instances
        .close_tab(&running.id, &second.id, TabCloseReason::Client)
        .await
        .unwrap();
    let view = app.instances.view(&running.id).unwrap();
    assert_eq!(view.tabs.len(), 1);
    assert_eq!(view.instance.state, InstanceState::Running);

    let missing = app
        .instances
        .close_tab(&running.id, &second.id, TabCloseReason::Client)
        .await;
    assert!(matches!(missing, Err(Error::NotFound { .. })));

    app.instances
        .stop(&app, &running.id, EndReason::Stop)
        .await
        .unwrap();
}

#[tokio::test]
async fn opened_tabs_reach_ready_without_a_navigation() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let (running, _first_tab) = app.instances.launch(&app, request(&profile)).await.unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    let mut opened = Vec::new();
    for url in ["data:text/html,<title>two</title>", "about:blank"] {
        opened.push(
            app.instances
                .open_tab(&running.id, Some(url.to_owned()))
                .await
                .unwrap()
                .id,
        );
    }

    wait_for("opened tabs to be ready", async || {
        let view = app.instances.view(&running.id).unwrap();
        opened.iter().all(|id| {
            view.tabs
                .iter()
                .any(|t| t.id == *id && t.state == TabState::Ready)
        })
    })
    .await;

    app.instances
        .stop(&app, &running.id, EndReason::Stop)
        .await
        .unwrap();
}

async fn crash(app: &Arc<App>, instance_id: InstanceId) {
    let pid = row(app, instance_id).await.pid.unwrap();
    kill(Pid::from_raw(i32::try_from(pid).unwrap()), Signal::SIGKILL).unwrap();
}

#[tokio::test]
async fn crash_detected() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let (running, only_tab) = app.instances.launch(&app, request(&profile)).await.unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    let client = CallClient {
        client_id: brr_core::ClientId::new(),
        kind: brr_core::ClientKind::Mcp,
        label: "crash test".to_owned(),
        implicit: true,
    };
    app.leases
        .acquire_tab(&app, &client, &running.id, &only_tab.id)
        .await
        .unwrap();

    crash(&app, running.id).await;
    let ended = wait_final(&app, running.id).await;
    wait_for("the last transition event", async || {
        triggers(&app, running.id).await.last().map(String::as_str) == Some("no_restart")
    })
    .await;

    assert_eq!(ended.state, InstanceState::Stopped);
    assert_eq!(ended.end_reason, Some(EndReason::Crashed));
    assert!(ended.error.is_some_and(|error| error.contains("signal 9")));
    assert_eq!(
        triggers(&app, running.id).await,
        [
            "launch",
            "admitted",
            "ready",
            "unexpected_exit",
            "no_restart"
        ]
    );

    let tabs = app
        .store
        .run(move |c| tab::list(c, &running.id, true))
        .await
        .unwrap();
    assert!(
        tabs.iter()
            .all(|t| t.state == TabState::Closed && t.closed_at.is_some())
    );

    let released = events_of(&app, LEASE_RELEASED, running.id).await;
    assert_eq!(released.len(), 1);
    assert_eq!(released[0]["reason"], "instance_crashed");
    assert!(app.leases.list(Some(&running.id), None).await.is_empty());
    assert!(marked_pids(running.id).is_empty());
}

#[tokio::test]
async fn auto_restart_relaunches() {
    let Some((_dir, app, build_id)) = chrome_app(&[AUTO_RESTART_ENV]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let (running, _) = app.instances.launch(&app, request(&profile)).await.unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    let first_pid = running.pid.unwrap();

    crash(&app, running.id).await;
    wait_for("the restarted browser", async || {
        let view = app.instances.view(&running.id).unwrap();

        view.instance.restart_count == 1
            && view.instance.state == InstanceState::Running
            && view.instance.pid != Some(first_pid)
            && view.tabs.iter().any(|t| t.state == TabState::Ready)
    })
    .await;

    let stopped = app
        .instances
        .stop(&app, &running.id, EndReason::Stop)
        .await
        .unwrap();
    assert_eq!(stopped.state, InstanceState::Stopped);
    assert_eq!(
        triggers(&app, running.id).await,
        [
            "launch",
            "admitted",
            "ready",
            "unexpected_exit",
            "auto_restart",
            "ready",
            "stop",
            "exited"
        ]
    );
}

#[tokio::test]
async fn graceful_shutdown() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let (first, _) = app.instances.launch(&app, request(&profile)).await.unwrap();
    let (second, _) = app.instances.launch(&app, request(&profile)).await.unwrap();
    let _guard = KillOnDrop::new(&app, vec![first.id, second.id]);
    let data_dirs = [
        first.data_dir.clone().unwrap(),
        second.data_dir.clone().unwrap(),
    ];

    let stop_wait = Duration::from_secs(app.settings().instance.stop_timeout_s + 5);
    time::timeout(stop_wait, app.shutdown()).await.unwrap();

    for instance_id in [first.id, second.id] {
        let ended = row(&app, instance_id).await;
        assert_eq!(ended.state, InstanceState::Stopped);
        assert_eq!(ended.end_reason, Some(EndReason::Shutdown));
        assert!(marked_pids(instance_id).is_empty());
    }
    assert!(data_dirs.iter().all(|dir| !dir.exists()));
}

#[tokio::test]
async fn open_tab_on_unknown_instance_is_not_found() {
    let (_dir, app) = scratch_app().await;

    let missing = app.instances.open_tab(&InstanceId::new(), None).await;

    assert!(matches!(missing, Err(Error::NotFound { .. })));
}

const FAIL_WRITES_TO: &str = "
    CREATE TRIGGER fail_writes BEFORE UPDATE ON instance WHEN NEW.state = '{state}'
    BEGIN SELECT RAISE(ABORT, 'disk full'); END;";

async fn fail_writes_to(app: &Arc<App>, state: &str) {
    let sql = FAIL_WRITES_TO.replace("{state}", state);
    app.store
        .run(move |c| {
            c.execute_batch(&sql).unwrap();

            Ok(())
        })
        .await
        .unwrap();
}

async fn heal_writes(app: &Arc<App>) {
    app.store
        .run(|c| {
            c.execute_batch("DROP TRIGGER fail_writes;").unwrap();

            Ok(())
        })
        .await
        .unwrap();
}

fn reject(profile: &Profile) -> LaunchRequest {
    LaunchRequest {
        admission: AdmissionMode::Reject,
        ..request(profile)
    }
}

// The first instance id the bus announced since `events` was subscribed.
fn announced_instance(
    events: &mut tokio::sync::broadcast::Receiver<Arc<brr_daemon::services::events::Published>>,
) -> InstanceId {
    loop {
        let published = events.try_recv().unwrap();
        if published.event.kind == STATE_CHANGED {
            return published.event.data["instance_id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
        }
    }
}

#[tokio::test]
async fn failed_admitted_write_frees_the_slot() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[ONE_INSTANCE])).await.unwrap();
    let build_id = register_build(&app, std::path::Path::new(NO_BROWSER)).await;
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let mut events = app.events.subscribe();
    fail_writes_to(&app, "starting").await;

    let failed = app.instances.launch(&app, request(&profile)).await;
    heal_writes(&app).await;

    assert!(failed.is_err());
    let failed_id = announced_instance(&mut events);
    assert_eq!(row(&app, failed_id).await.state, InstanceState::Failed);
    // The one slot is free: this launch is not turned away, it fails on the missing browser.
    let next = app.instances.launch(&app, reject(&profile)).await;
    assert!(!matches!(next, Err(Error::CapExceeded { .. })), "{next:?}");
}

#[tokio::test]
async fn failed_ready_write_frees_the_slot_and_the_profile_dir() {
    let Some((_dir, app, build_id)) = chrome_app(&[ONE_INSTANCE]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let mut events = app.events.subscribe();
    fail_writes_to(&app, "running").await;

    let failed = app.instances.launch(&app, reject(&profile)).await;
    heal_writes(&app).await;

    assert!(failed.is_err());
    let failed_id = announced_instance(&mut events);
    let _guard = KillOnDrop::new(&app, vec![failed_id]);
    assert_eq!(row(&app, failed_id).await.state, InstanceState::Failed);
    assert!(
        !app.paths
            .ephemeral_dir()
            .join(failed_id.to_string())
            .exists()
    );
    assert!(marked_pids(failed_id).is_empty());

    let (next, _) = app.instances.launch(&app, reject(&profile)).await.unwrap();
    app.instances
        .stop(&app, &next.id, EndReason::Stop)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_failed_store_write_does_not_keep_the_browser_alive_on_stop() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let (running, _) = app.instances.launch(&app, request(&profile)).await.unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    fail_writes_to(&app, "stopping").await;

    let stopped = app
        .instances
        .stop(&app, &running.id, EndReason::Stop)
        .await
        .unwrap();

    assert_eq!(stopped.state, InstanceState::Stopped);
    assert!(marked_pids(running.id).is_empty());
}

#[tokio::test]
async fn stop_reaches_a_launch_that_is_still_starting() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let mut events = app.events.subscribe();
    let launch = {
        let (app, request) = (Arc::clone(&app), request(&profile));
        tokio::spawn(async move { app.instances.launch(&app, request).await })
    };
    let mut instance_id = None;
    wait_for("the launch to be announced", async || {
        instance_id = events.try_recv().ok().map(|published| {
            published.event.data["instance_id"]
                .as_str()
                .unwrap()
                .parse::<InstanceId>()
                .unwrap()
        });
        instance_id.is_some()
    })
    .await;
    let instance_id = instance_id.unwrap();
    let _guard = KillOnDrop::new(&app, vec![instance_id]);

    let stopped = app
        .instances
        .stop(&app, &instance_id, EndReason::Stop)
        .await
        .unwrap();

    assert_eq!(stopped.state, InstanceState::Stopped);
    assert_eq!(stopped.end_reason, Some(EndReason::Stop));
    assert!(launch.await.unwrap().is_err());
    assert!(marked_pids(instance_id).is_empty());
}

#[tokio::test]
async fn shutdown_refuses_new_launches_and_attaches() {
    let (_dir, app) = scratch_app().await;

    app.instances.shutdown_all(&app).await;

    let profile = profile_row(ProfileKind::Ephemeral, None, None);
    let launched = app.instances.launch(&app, request(&profile)).await;
    let attached = app
        .instances
        .attach(&app, AttachSource::CdpUrl("http://127.0.0.1:1".to_owned()))
        .await;
    assert!(matches!(launched, Err(Error::DependencyMissing { .. })));
    assert!(matches!(attached, Err(Error::DependencyMissing { .. })));
}

// The renderer cannot start, so the first page never becomes ready.
#[tokio::test]
async fn start_timeout_ends_the_instance_failed() {
    let Some((_dir, app, build_id)) = chrome_app(&[START_TIMEOUT]).await else {
        return;
    };
    let mut profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    profile.settings.args.push(NO_RENDERER.to_owned());
    let mut events = app.events.subscribe();

    let launched = app.instances.launch(&app, request(&profile)).await;

    assert!(
        matches!(launched, Err(Error::Timeout { .. })),
        "{launched:?}"
    );
    let instance_id = announced_instance(&mut events);
    let _guard = KillOnDrop::new(&app, vec![instance_id]);
    let ended = wait_final(&app, instance_id).await;
    assert_eq!(ended.state, InstanceState::Failed);
    assert_eq!(ended.end_reason, Some(EndReason::StartTimeout));
    assert_eq!(
        triggers(&app, instance_id).await,
        ["launch", "admitted", "start_timeout"]
    );
    assert!(
        !app.paths
            .ephemeral_dir()
            .join(instance_id.to_string())
            .exists()
    );
    assert!(marked_pids(instance_id).is_empty());
}

#[tokio::test]
async fn a_second_stop_after_a_cancelled_launch_returns_at_once() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let mut events = app.events.subscribe();
    let launch = {
        let (app, request) = (Arc::clone(&app), request(&profile));
        tokio::spawn(async move { app.instances.launch(&app, request).await })
    };
    let mut instance_id = None;
    wait_for("the launch to be announced", async || {
        instance_id = events
            .try_recv()
            .ok()
            .filter(|published| published.event.kind == STATE_CHANGED)
            .map(|published| {
                published.event.data["instance_id"]
                    .as_str()
                    .unwrap()
                    .parse::<InstanceId>()
                    .unwrap()
            });
        instance_id.is_some()
    })
    .await;
    let instance_id = instance_id.unwrap();
    let _guard = KillOnDrop::new(&app, vec![instance_id]);

    app.instances
        .stop(&app, &instance_id, EndReason::Stop)
        .await
        .unwrap();
    // The launch task may still be winding down: the second stop must not wait for an event
    // that already happened.
    let again = time::timeout(
        STOP_AGAIN_TIMEOUT,
        app.instances.stop(&app, &instance_id, EndReason::Stop),
    )
    .await;

    assert!(again.is_ok(), "the second stop did not return");
    let _ = launch.await;
}

#[tokio::test]
async fn shutdown_leaves_no_launch_behind() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    let mut events = app.events.subscribe();
    let launch = {
        let (app, request) = (Arc::clone(&app), request(&profile));
        tokio::spawn(async move { app.instances.launch(&app, request).await })
    };

    app.instances.shutdown_all(&app).await;
    let _ = launch.await;

    assert!(app.instances.views().is_empty());
    while let Ok(published) = events.try_recv() {
        if published.event.kind == STATE_CHANGED {
            let instance_id: InstanceId = published.event.data["instance_id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
            assert!(marked_pids(instance_id).is_empty());
        }
    }
}

#[tokio::test]
async fn failed_start_with_a_failing_write_still_ends_the_owner() {
    let Some((_dir, app, build_id)) = chrome_app(&[START_TIMEOUT]).await else {
        return;
    };
    let mut profile = add_profile(&app, build_id, ProfileKind::Ephemeral, "scratch").await;
    profile.settings.args.push(NO_RENDERER.to_owned());
    let mut events = app.events.subscribe();
    fail_writes_to(&app, "failed").await;

    let launched = app.instances.launch(&app, request(&profile)).await;
    heal_writes(&app).await;

    assert!(
        matches!(launched, Err(Error::Timeout { .. })),
        "{launched:?}"
    );
    let instance_id = announced_instance(&mut events);
    let _guard = KillOnDrop::new(&app, vec![instance_id]);
    wait_for("the owner to end", async || {
        app.instances.view(&instance_id).is_none()
    })
    .await;
    let stopped = time::timeout(
        STOP_AGAIN_TIMEOUT,
        app.instances.stop(&app, &instance_id, EndReason::Stop),
    )
    .await;
    assert!(stopped.is_ok(), "a stop after the failed start hung");
    assert!(marked_pids(instance_id).is_empty());
}
