#![allow(clippy::unwrap_used)]

use crate::common;

use std::collections::BTreeMap;
use std::{
    fs,
    io::{Cursor, Write},
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{Router, routing::get};
use brr_core::{
    Build, BuildChoice, BuildId, BuildSource, BuildState, Channel, ConflictReason, Engine, Error,
    Event, Paths, ProfileSettings, now_ms,
};
use brr_daemon::{
    App, ops,
    services::{builds::channels, profiles::resolve_build},
};
use brr_store::{Store, build};
use serde_json::{Value, json};
use test_case::test_case;
use tokio::{net::TcpListener, time};
use zip::{ZipWriter, write::SimpleFileOptions};

use common::{boot, ctx, paths, scratch_app};

const CHROME_VERSION: &str = "157.0.1.1";
const NEXT_CHROME_VERSION: &str = "158.0.1.1";
const UNGOOGLED_VERSION: &str = "156.0.8078.79";
const WRONG_DIGEST: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";
const SYSTEM_VERSION: &str = "152.0.1.2";
const EXECUTABLE_MODE: u32 = 0o755;
const ZIP_DELAY: Duration = Duration::from_millis(200);
const POLL_STEP: Duration = Duration::from_millis(20);
const POLL_TIMEOUT: Duration = Duration::from_secs(10);
const TEMPLATE_NAME: &str = "pinned-template";

struct Cft {
    base: String,
    version: Arc<Mutex<String>>,
    zip_gets: Arc<AtomicUsize>,
}

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    let op = ops::find(name).unwrap();

    ops::dispatch(ctx(app), op, input).await
}

fn chrome_zip() -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    // The folder depends on the host architecture, so ship both.
    for entry in ["chrome-linux64/chrome", "chrome-linux-arm64/chrome"] {
        writer
            .start_file(
                entry,
                SimpleFileOptions::default().unix_permissions(EXECUTABLE_MODE),
            )
            .unwrap();
        writer.write_all(b"#!/bin/sh\n").unwrap();
    }

    writer.finish().unwrap().into_inner()
}

fn cft_release(version: &str, url: &str) -> Value {
    json!({
        "version": version,
        "downloads": {"chrome": [
            {"platform": "linux64", "url": url},
            {"platform": "linux-arm64", "url": url},
        ]},
    })
}

async fn serve_cft() -> Cft {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let url = format!("{base}/chrome.zip");
    let version = Arc::new(Mutex::new(CHROME_VERSION.to_string()));

    let zip = chrome_zip();
    let zip_gets = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&zip_gets);
    let (stable, known) = (Arc::clone(&version), Arc::clone(&version));
    let (stable_url, known_url) = (url.clone(), url);
    let router = Router::new()
        .route(
            "/last-known-good-versions-with-downloads.json",
            get(move || async move {
                let release = cft_release(&stable.lock().unwrap(), &stable_url);

                json!({"channels": {"Stable": release}}).to_string()
            }),
        )
        .route(
            "/known-good-versions-with-downloads.json",
            get(move || async move {
                let release = cft_release(&known.lock().unwrap(), &known_url);

                json!({"versions": [release]}).to_string()
            }),
        )
        .route(
            "/chrome.zip",
            get(move || async move {
                counter.fetch_add(1, Ordering::SeqCst);
                time::sleep(ZIP_DELAY).await;

                zip
            }),
        );
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    Cft {
        base,
        version,
        zip_gets,
    }
}

async fn app_with_cft() -> (tempfile::TempDir, Arc<App>, Cft) {
    let cft = serve_cft().await;
    let dir = tempfile::tempdir().unwrap();
    let mut boot = boot(dir.path(), &[]);
    boot.cft_base_url = cft.base.clone();
    let app = App::start(boot).await.unwrap();

    (dir, app, cft)
}

async fn events_of(app: &Arc<App>, kind: &str) -> Vec<Event> {
    let listed = call(app, "events.list", json!({"kinds": kind}))
        .await
        .unwrap();

    serde_json::from_value(listed["events"].clone()).unwrap()
}

async fn wait_events(app: &Arc<App>, kind: &str, count: usize) {
    time::timeout(POLL_TIMEOUT, async {
        while events_of(app, kind).await.len() < count {
            time::sleep(POLL_STEP).await;
        }
    })
    .await
    .unwrap();
}

async fn wait_installed(app: &Arc<App>) {
    wait_events(app, "build.installed", 1).await;
}

fn invalid_field(result: Result<Value, Error>) -> String {
    match result {
        Err(Error::InvalidArgument { field, .. }) => field,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[tokio::test]
async fn detect_registers_once() {
    let bin = tempfile::tempdir().unwrap();
    // Chrome's vendor install at /opt may exist on the host and hides the google-chrome wrappers.
    let script = bin.path().join("chromium");
    fs::write(
        &script,
        format!("#!/bin/sh\necho 'Chromium {SYSTEM_VERSION}'\n"),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut boot = boot(dir.path(), &[]);
    boot.path_env = bin.path().into();
    let app = App::start(boot).await.unwrap();

    call(&app, "build.detect", json!({})).await.unwrap();
    call(&app, "build.detect", json!({})).await.unwrap();

    let listed = call(&app, "build.list", json!({})).await.unwrap();
    let builds: Vec<Build> = serde_json::from_value(listed["builds"].clone()).unwrap();
    let ours: Vec<&Build> = builds
        .iter()
        .filter(|b| b.version == SYSTEM_VERSION)
        .collect();
    assert_eq!(ours.len(), 1);
    assert_eq!(ours[0].source, BuildSource::System);
    assert_eq!(ours[0].state, BuildState::Registered);
    assert_eq!(ours[0].path, script.canonicalize().unwrap());
}

#[tokio::test]
async fn install_cft_flow() {
    let (_dir, app, cft) = app_with_cft().await;
    let mut bus = app.events.subscribe();

    let started: Build = serde_json::from_value(
        call(
            &app,
            "build.install",
            json!({"source": "cft", "channel": "stable"}),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(started.state, BuildState::Installing);
    assert_eq!(started.version, CHROME_VERSION);
    assert_eq!(started.channel.as_deref(), Some("stable"));

    wait_installed(&app).await;

    let done: Build = serde_json::from_value(
        call(&app, "build.get", json!({"build_id": started.id}))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(done.state, BuildState::Installed);
    assert_eq!(done.engine, Engine::Chromium);
    assert!(done.sha256.is_some());
    assert!(done.installed_at.is_some());
    assert!(done.path.is_file());
    let published = events_of(&app, "build.installed").await;
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].kind, "build.installed");

    let mut kinds = Vec::new();
    while let Ok(event) = bus.try_recv() {
        kinds.push(event.event.kind.clone());
    }
    assert!(kinds.iter().any(|k| k == "build.install_progress"));
    assert_eq!(cft.zip_gets.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn install_single_flight() {
    let (_dir, app, cft) = app_with_cft().await;
    let input = json!({"source": "cft", "version": CHROME_VERSION});

    let (first, second) = tokio::join!(
        call(&app, "build.install", input.clone()),
        call(&app, "build.install", input)
    );
    let first: Build = serde_json::from_value(first.unwrap()).unwrap();
    let second: Build = serde_json::from_value(second.unwrap()).unwrap();
    wait_installed(&app).await;

    assert_eq!(first.id, second.id);
    assert_eq!(first.state, BuildState::Installing);
    let listed = call(&app, "build.list", json!({})).await.unwrap();
    assert_eq!(listed["builds"].as_array().unwrap().len(), 1);
    assert_eq!(cft.zip_gets.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn install_validation() {
    let (_dir, app) = scratch_app().await;

    let system = call(&app, "build.install", json!({"source": "system"})).await;
    let beta = call(
        &app,
        "build.install",
        json!({"source": "brave", "channel": "beta"}),
    )
    .await;
    let both = call(
        &app,
        "build.install",
        json!({"source": "cft", "channel": "stable", "version": "1.2"}),
    )
    .await;

    assert_eq!(invalid_field(system), "source");
    assert_eq!(invalid_field(beta), "channel");
    assert_eq!(invalid_field(both), "channel");
}

#[tokio::test]
async fn available_lists_the_stable_channel() {
    let (_dir, app, _cft) = app_with_cft().await;

    let listed = call(&app, "build.available", json!({})).await.unwrap();

    let stable = listed["available"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["channel"] == "stable")
        .unwrap();
    assert_eq!(stable["version"], CHROME_VERSION);
}

fn installing_row() -> Build {
    Build {
        id: BuildId::new(),
        source: BuildSource::Cft,
        engine: Engine::Chromium,
        name: "Chrome for Testing".to_string(),
        version: CHROME_VERSION.to_string(),
        channel: None,
        path: Path::new("/nonexistent").to_path_buf(),
        state: BuildState::Installing,
        sha256: None,
        size_bytes: 0,
        capabilities: Vec::new(),
        installed_at: Some(now_ms()),
        last_referenced_at: None,
    }
}

#[tokio::test]
async fn interrupted_marked_failed() {
    let dir = tempfile::tempdir().unwrap();
    let paths: Paths = paths(dir.path());
    fs::create_dir_all(&paths.data_dir).unwrap();
    let row = installing_row();
    let store = Store::open(&paths.db_file()).unwrap();
    let inserted = row.clone();
    store
        .run(move |c| build::insert(c, &inserted))
        .await
        .unwrap();
    drop(store);

    let app = App::start(boot(dir.path(), &[])).await.unwrap();

    let found = call(&app, "build.get", json!({"build_id": row.id}))
        .await
        .unwrap();
    assert_eq!(found["state"], "failed");
}

fn tar_xz(version: &str) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join(format!("ungoogled-chromium-{version}-1_linux"));
    fs::create_dir_all(&root).unwrap();
    let chrome = root.join("chrome");
    fs::write(&chrome, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&chrome, fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
    let out = dir.path().join("bundle.tar.xz");
    let status = Command::new("tar")
        .arg("-cJf")
        .arg(&out)
        .arg("-C")
        .arg(dir.path())
        .arg(root.file_name().unwrap())
        .status()
        .unwrap();
    assert!(status.success());

    fs::read(out).unwrap()
}

#[allow(clippy::disallowed_macros)]
fn has_xz() -> bool {
    let found = Command::new("xz").arg("--version").output().is_ok();
    if !found {
        eprintln!("skipped: xz is not on PATH");
    }

    found
}

fn sha256_of(bytes: &[u8]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("archive");
    fs::write(&file, bytes).unwrap();
    let out = Command::new("sha256sum").arg(&file).output().unwrap();

    String::from_utf8(out.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

// A GitHub API stand-in publishing one ungoogled release; `digest` None publishes the real one.
async fn app_with_ungoogled(digest: Option<&str>) -> (tempfile::TempDir, Arc<App>) {
    let archive = tar_xz(UNGOOGLED_VERSION);
    let digest = digest.map_or_else(|| format!("sha256:{}", sha256_of(&archive)), str::to_string);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let assets: Vec<Value> = ["x86_64", "arm64"]
        .into_iter()
        .map(|arch| {
            json!({
                "name": format!("ungoogled-chromium-{UNGOOGLED_VERSION}-1-{arch}_linux.tar.xz"),
                "browser_download_url": format!("{base}/ungoogled.tar.xz"),
                "digest": digest,
            })
        })
        .collect();
    let releases = json!([{
        "tag_name": format!("{UNGOOGLED_VERSION}-1"),
        "draft": false,
        "prerelease": false,
        "assets": assets,
    }])
    .to_string();
    let router = Router::new()
        .route(
            "/repos/ungoogled-software/ungoogled-chromium-portablelinux/releases",
            get(move || async move { releases }),
        )
        .route("/ungoogled.tar.xz", get(move || async move { archive }));
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[("BRR_BUILDS__GITHUB_API", &base)]))
        .await
        .unwrap();

    (dir, app)
}

#[tokio::test]
async fn install_github_source() {
    if !has_xz() {
        return;
    }
    let (_dir, app) = app_with_ungoogled(None).await;

    let started: Build = serde_json::from_value(
        call(
            &app,
            "build.install",
            json!({"source": "ungoogled", "channel": "stable"}),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    wait_installed(&app).await;

    let done: Build = serde_json::from_value(
        call(&app, "build.get", json!({"build_id": started.id}))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(done.state, BuildState::Installed);
    assert_eq!(done.name, "ungoogled-chromium");
    assert_eq!(done.version, UNGOOGLED_VERSION);
    assert!(
        done.capabilities
            .contains(&brr_core::Capability::UnpackedExtensions)
    );
    assert!(done.path.is_file());
}

#[tokio::test]
async fn checksum_mismatch_fails() {
    if !has_xz() {
        return;
    }
    let (_dir, app) = app_with_ungoogled(Some(WRONG_DIGEST)).await;

    let started: Build = serde_json::from_value(
        call(&app, "build.install", json!({"source": "ungoogled"}))
            .await
            .unwrap(),
    )
    .unwrap();
    wait_events(&app, "build.install_failed", 1).await;

    let failed = events_of(&app, "build.install_failed").await;
    assert!(
        failed[0].data["error"]
            .as_str()
            .unwrap()
            .contains("checksum mismatch")
    );
    let row = call(&app, "build.get", json!({"build_id": started.id}))
        .await
        .unwrap();
    assert_eq!(row["state"], "failed");
}

async fn create_profile(app: &Arc<App>, name: &str) -> Value {
    call(
        app,
        "profile.create",
        json!({"name": name, "template": "chromium-headless"}),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn channel_moves() {
    let (_dir, app, cft) = app_with_cft().await;
    call(&app, "build.install", json!({"source": "cft"}))
        .await
        .unwrap();
    wait_events(&app, "build.installed", 1).await;
    let profile = create_profile(&app, "follower").await;
    assert_eq!(profile["settings"]["build"]["channel"], "cft:stable");

    *cft.version.lock().unwrap() = NEXT_CHROME_VERSION.to_string();
    let results = channels::check_all(&app).await;
    wait_events(&app, "build.installed", 2).await;

    assert!(results.iter().all(Result::is_ok));
    let moved = events_of(&app, "build.channel_moved").await;
    let last = moved
        .iter()
        .find(|e| e.data["to"] == NEXT_CHROME_VERSION)
        .unwrap();
    assert_eq!(last.data["source"], "cft");
    assert_eq!(last.data["channel"], "stable");
    assert_eq!(last.data["from"], CHROME_VERSION);
    assert_eq!(last.data["to"], NEXT_CHROME_VERSION);
    let choice = BuildChoice::Channel(Channel {
        source: BuildSource::Cft,
        name: "stable".to_string(),
    });
    let resolved = app
        .store
        .run(move |c| resolve_build(c, &choice))
        .await
        .unwrap();
    assert_eq!(resolved.version, NEXT_CHROME_VERSION);
}

fn conflict_referenced(result: Result<Value, Error>) -> String {
    match result {
        Err(Error::Conflict {
            reason: ConflictReason::Referenced,
            detail,
        }) => detail.unwrap(),
        other => panic!("expected Conflict(referenced), got {other:?}"),
    }
}

#[tokio::test]
async fn remove_rules() {
    let (_dir, app, _cft) = app_with_cft().await;
    // The built-in oneshot Profile follows the stable channel, which would pin the Build: another
    // Profile takes its place, which makes it an ordinary one to delete.
    app.set_settings(BTreeMap::from([(
        "oneshot.profile".to_string(),
        json!("elsewhere"),
    )]))
    .await
    .unwrap();
    call(&app, "profile.delete", json!({"profile": "oneshot"}))
        .await
        .unwrap();
    let installed: Build = serde_json::from_value(
        call(&app, "build.install", json!({"source": "cft"}))
            .await
            .unwrap(),
    )
    .unwrap();
    let busy = call(&app, "build.remove", json!({"build_id": installed.id})).await;
    wait_installed(&app).await;
    let done: Build = serde_json::from_value(
        call(&app, "build.get", json!({"build_id": installed.id}))
            .await
            .unwrap(),
    )
    .unwrap();
    let profile = create_profile(&app, "pinner").await;
    let mut settings = profile["settings"].clone();
    settings["build"] = json!({"build_id": installed.id});
    call(
        &app,
        "profile.update",
        json!({"profile": "pinner", "settings": settings}),
    )
    .await
    .unwrap();

    let pinned = call(&app, "build.remove", json!({"build_id": installed.id})).await;
    let view = call(&app, "build.get", json!({"build_id": installed.id}))
        .await
        .unwrap();
    call(
        &app,
        "profile.update",
        json!({"profile": "pinner", "settings": profile["settings"]}),
    )
    .await
    .unwrap();
    let channel_ref = call(&app, "build.get", json!({"build_id": installed.id}))
        .await
        .unwrap();
    call(&app, "profile.delete", json!({"profile": "pinner"}))
        .await
        .unwrap();
    let removed = call(&app, "build.remove", json!({"build_id": installed.id})).await;

    assert_eq!(conflict_referenced(busy), "install in progress");
    assert!(conflict_referenced(pinned).contains("pinner"));
    assert_eq!(
        view["referenced_by"],
        json!([{"kind": "profile", "id": profile["id"], "label": "pinner"}])
    );
    assert_eq!(
        channel_ref["referenced_by"],
        json!([{"kind": "channel", "id": "cft:stable", "label": "cft:stable"}])
    );
    removed.unwrap();
    assert!(!done.path.exists());
    assert_eq!(events_of(&app, "build.removed").await.len(), 1);
    let gone = call(&app, "build.get", json!({"build_id": installed.id})).await;
    assert!(matches!(gone, Err(Error::NotFound { .. })));
}

#[test_case(true; "pinned by id")]
#[test_case(false; "following its channel")]
#[tokio::test]
async fn remove_refuses_a_build_a_template_uses(pinned: bool) {
    let (_dir, app, _cft) = app_with_cft().await;
    app.set_settings(BTreeMap::from([(
        "oneshot.profile".to_string(),
        json!("elsewhere"),
    )]))
    .await
    .unwrap();
    call(&app, "profile.delete", json!({"profile": "oneshot"}))
        .await
        .unwrap();
    let installed: Build = serde_json::from_value(
        call(&app, "build.install", json!({"source": "cft"}))
            .await
            .unwrap(),
    )
    .unwrap();
    wait_installed(&app).await;
    let build = if pinned {
        BuildChoice::BuildId(installed.id)
    } else {
        BuildChoice::Channel(Channel {
            source: BuildSource::Cft,
            name: "stable".to_string(),
        })
    };
    let settings = serde_json::to_value(ProfileSettings {
        build,
        ..ProfileSettings::default()
    })
    .unwrap();
    let template = call(
        &app,
        "template.create",
        json!({"name": TEMPLATE_NAME, "settings": settings}),
    )
    .await
    .unwrap();

    let refused = call(&app, "build.remove", json!({"build_id": installed.id})).await;
    let view = call(&app, "build.get", json!({"build_id": installed.id}))
        .await
        .unwrap();
    call(&app, "template.delete", json!({"template": TEMPLATE_NAME}))
        .await
        .unwrap();
    let removed = call(&app, "build.remove", json!({"build_id": installed.id})).await;

    assert_eq!(
        conflict_referenced(refused),
        format!("template {TEMPLATE_NAME}")
    );
    assert_eq!(
        view["referenced_by"],
        json!([{"kind": "template", "id": template["id"], "label": TEMPLATE_NAME}])
    );
    removed.unwrap();
}

#[tokio::test]
async fn remove_refuses_system_builds() {
    let bin = tempfile::tempdir().unwrap();
    let script = bin.path().join("google-chrome-stable");
    fs::write(
        &script,
        format!("#!/bin/sh\necho 'Google Chrome {SYSTEM_VERSION}'\n"),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(EXECUTABLE_MODE)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut boot = boot(dir.path(), &[]);
    boot.path_env = bin.path().into();
    let app = App::start(boot).await.unwrap();
    let detected = call(&app, "build.detect", json!({})).await.unwrap();

    let removed = call(
        &app,
        "build.remove",
        json!({"build_id": detected["builds"][0]["id"]}),
    )
    .await;

    assert_eq!(invalid_field(removed), "build_id");
}

#[tokio::test]
async fn remove_path_guard() {
    let (_dir, app) = scratch_app().await;
    let outside = tempfile::tempdir().unwrap();
    let mut row = installing_row();
    row.state = BuildState::Installed;
    row.path = outside.path().to_path_buf();
    let inserted = row.clone();
    app.store
        .run(move |c| build::insert(c, &inserted))
        .await
        .unwrap();
    fs::create_dir_all(app.paths.builds_dir()).unwrap();

    let removed = call(&app, "build.remove", json!({"build_id": row.id})).await;

    assert!(matches!(removed, Err(Error::Internal { .. })));
    assert!(outside.path().is_dir());
}
