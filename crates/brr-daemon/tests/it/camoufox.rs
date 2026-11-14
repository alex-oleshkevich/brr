#![allow(clippy::unwrap_used)]

use crate::common;

use std::{
    convert::Infallible,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::{Body, Bytes},
    http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, SET_COOKIE},
    response::Html,
    routing::get,
};
use brr_camoufox::data::fpgen_pin;
use brr_core::{
    AdmissionMode, Asset, AssetId, AssetKind, Build, BuildChoice, BuildId, BuildSource, BuildState,
    Capability, Channel, Display, EndReason, Engine, Fingerprint, FingerprintOs, InstanceId, Order,
    Page, Profile, ProfileId, ProfileKind, ProfileSettings, WaitUntil, now_ms,
};
use brr_daemon::{
    App, ops,
    services::{
        builds::{current_for_channel, finish_install},
        instances::LaunchRequest,
    },
};
use brr_store::{artifact, asset, build, event, profile};
use serde_json::{Value, json};
use tempfile::TempDir;

use common::{KillOnDrop, boot, ctx, events_of, wait_for};

const CAMOUFOX_ENV: &str = "BRR_TEST_CAMOUFOX";
const MODEL_ENV: &str = "BRR_TEST_FPGEN_MODEL";
const CHANNEL: &str = "latest-compatible";
const VERSION: &str = "158.0.1-beta.41";
const COMPAT_FILE: &str = "brr-compat.json";
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(30);
const EVAL_TIMEOUT: Duration = Duration::from_secs(10);
const EPHEMERAL_LAUNCHES: usize = 5;
const DOWNLOAD_BYTES: usize = 3 * 1024 * 1024;
const DOWNLOAD_CHUNK: usize = 64 * 1024;
const COOKIE: &str = "fox=biscuit";
const BOOKMARKS_HTML: &str = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF="https://one.example/">One</A><DT><A HREF="https://two.example/">Two</A></DL><p>"#;

struct Assets {
    binary: PathBuf,
    model: PathBuf,
}

fn assets() -> Option<Assets> {
    let Some(binary) = std::env::var_os(CAMOUFOX_ENV).map(PathBuf::from) else {
        eprintln!("skipped: {CAMOUFOX_ENV} is not set");
        return None;
    };
    let Some(model) = std::env::var_os(MODEL_ENV).map(PathBuf::from) else {
        eprintln!("skipped: {MODEL_ENV} is not set");
        return None;
    };

    Some(Assets { binary, model })
}

async fn app_with_assets(dir: &TempDir, assets: &Assets, env: &[(&str, &str)]) -> Arc<App> {
    let app = App::start(boot(dir.path(), env)).await.unwrap();
    // The GeoIP row only has to exist: nothing here opens the database.
    let database = dir.path().join("geoip.mmdb");
    fs::write(&database, b"").unwrap();
    let fonts = assets.binary.parent().unwrap().join("fonts");
    let model = assets.model.clone();
    let row = |kind, version: &str, path| Asset {
        id: AssetId::new(),
        kind,
        version: version.to_owned(),
        path,
        updated_at: now_ms(),
    };
    let rows = [
        row(AssetKind::FontsBundle, "v1", fonts),
        row(AssetKind::FpgenModel, &fpgen_pin().sha256, model),
        row(AssetKind::GeoipDb, "test", database),
    ];
    app.store
        .run(move |c| rows.iter().try_for_each(|row| asset::put(c, row)))
        .await
        .unwrap();

    app
}

// The suite writes its record next to the executable: a symlinked copy of the install keeps the
// shared test install untouched.
fn mirror_install(dir: &TempDir, assets: &Assets) -> (PathBuf, PathBuf) {
    let source = assets.binary.parent().unwrap();
    let mirror = dir.path().join("camoufox-mirror");
    fs::create_dir_all(&mirror).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let name = entry.unwrap().file_name();
        if name != COMPAT_FILE {
            std::os::unix::fs::symlink(source.join(&name), mirror.join(&name)).unwrap();
        }
    }
    let binary = mirror.join(assets.binary.file_name().unwrap());

    (mirror, binary)
}

fn build_row(path: PathBuf, channel: Option<&str>) -> Build {
    Build {
        id: BuildId::new(),
        source: BuildSource::Camoufox,
        engine: Engine::Camoufox,
        name: "Camoufox".to_owned(),
        version: VERSION.to_owned(),
        channel: channel.map(str::to_owned),
        path,
        state: BuildState::Installing,
        sha256: None,
        size_bytes: 0,
        capabilities: vec![Capability::Screencast],
        installed_at: None,
        last_referenced_at: None,
    }
}

async fn insert_build(app: &Arc<App>, row: &Build) {
    let row = row.clone();
    app.store
        .run(move |c| build::insert(c, &row))
        .await
        .unwrap();
}

async fn builds_of(app: &Arc<App>) -> Vec<Build> {
    app.store.run(|c| build::all(c)).await.unwrap()
}

fn latest_compatible() -> Channel {
    Channel {
        source: BuildSource::Camoufox,
        name: CHANNEL.to_owned(),
    }
}

async fn failed_events(app: &Arc<App>) -> Vec<Value> {
    let query = event::Query {
        since: None,
        kinds: vec!["build.install_failed".to_owned()],
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: Order::Asc,
        page: Page::new(Some(100), None),
    };
    let (events, _) = app
        .store
        .run(move |c| event::list(c, &query))
        .await
        .unwrap();

    events.into_iter().map(|event| event.data).collect()
}

#[tokio::test]
async fn failing_release_never_current() {
    let Some(assets) = assets() else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let app = app_with_assets(&dir, &assets, &[]).await;
    let install = dir.path().join("camoufox");
    fs::create_dir_all(&install).unwrap();
    let binary = install.join("camoufox-bin");
    fs::copy("/bin/false", &binary).unwrap();
    let row = build_row(binary, Some(CHANNEL));
    insert_build(&app, &row).await;

    let finished = finish_install(&app, row.clone(), None).await.unwrap();

    assert_eq!(finished.channel, None);
    let record: Value =
        serde_json::from_slice(&fs::read(install.join(COMPAT_FILE)).unwrap()).unwrap();
    assert_eq!(record["passed"], false);
    assert_eq!(record["failed_step"], "launch");
    let stored = builds_of(&app).await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].channel, None);
    let failures = failed_events(&app).await;
    assert!(
        failures[0]["error"]
            .as_str()
            .unwrap()
            .starts_with("compatibility suite failed at launch"),
        "{failures:?}"
    );
    assert_eq!(current_for_channel(&stored, &latest_compatible()), None);
}

#[tokio::test]
async fn passing_release_becomes_current() {
    let Some(assets) = assets() else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let app = app_with_assets(&dir, &assets, &[]).await;
    let (install, binary) = mirror_install(&dir, &assets);
    let row = build_row(binary, Some(CHANNEL));
    insert_build(&app, &row).await;

    finish_install(&app, row.clone(), None).await.unwrap();

    let record: Value =
        serde_json::from_slice(&fs::read(install.join(COMPAT_FILE)).unwrap()).unwrap();
    assert_eq!(record["passed"], true, "{record}");
    let current = current_for_channel(&builds_of(&app).await, &latest_compatible()).unwrap();
    assert_eq!(current.id, row.id);
}

#[tokio::test]
async fn stale_record_reruns_suite() {
    let Some(assets) = assets() else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let app = app_with_assets(&dir, &assets, &[]).await;
    let (install, binary) = mirror_install(&dir, &assets);
    // A pass under another brr version would stand in for the suite if it were trusted.
    fs::write(
        install.join(COMPAT_FILE),
        br#"{"passed":true,"checked_at":0,"failed_step":null,"brr_version":"0.0.0-old"}"#,
    )
    .unwrap();
    let row = build_row(binary, Some(CHANNEL));
    insert_build(&app, &row).await;

    finish_install(&app, row.clone(), None).await.unwrap();

    let record: Value =
        serde_json::from_slice(&fs::read(install.join(COMPAT_FILE)).unwrap()).unwrap();
    assert_eq!(record["brr_version"], app.version, "{record}");
    assert_ne!(record["checked_at"], 0);
}

async fn camoufox_profile(
    app: &Arc<App>,
    build_id: BuildId,
    kind: ProfileKind,
    name: &str,
) -> Profile {
    let id = ProfileId::new();
    let data_dir = (kind == ProfileKind::Persistent).then(|| {
        let dir = app.paths.profiles_dir().join(id.to_string());
        fs::create_dir_all(&dir).unwrap();

        dir
    });
    let now = now_ms();
    let created = Profile {
        id,
        name: name.to_owned(),
        kind,
        template_id: None,
        settings: ProfileSettings {
            build: BuildChoice::BuildId(build_id),
            display: Display::Headless,
            fingerprint: Some(Fingerprint {
                os: Some(FingerprintOs::Linux),
                ..Fingerprint::default()
            }),
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
    let stored = created.clone();
    app.store
        .run(move |c| profile::insert(c, &stored))
        .await
        .unwrap();

    created
}

async fn registered(app: &Arc<App>, assets: &Assets) -> BuildId {
    let mut row = build_row(assets.binary.clone(), None);
    row.state = BuildState::Installed;
    insert_build(app, &row).await;

    row.id
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

// Launches, reads the identity the page sees, stops.
async fn identity(app: &Arc<App>, guard: &KillOnDrop, profile: &Profile) -> Value {
    let (instance, tab) = app.instances.launch(app, request(profile)).await.unwrap();
    guard.track(instance.id);
    let page = app.instances.page(&instance.id, &tab.id).await.unwrap();
    let seen = page
        .evaluate(
            "[navigator.userAgent, screen.width, screen.height]",
            EVAL_TIMEOUT,
        )
        .await
        .unwrap();
    app.instances
        .stop(app, &instance.id, EndReason::Stop)
        .await
        .unwrap();

    seen
}

#[tokio::test]
async fn stable_persistent_identity() {
    let Some(assets) = assets() else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let app = app_with_assets(&dir, &assets, &[]).await;
    let build_id = registered(&app, &assets).await;
    let guard = KillOnDrop::new(&app, Vec::new());

    let persistent = camoufox_profile(&app, build_id, ProfileKind::Persistent, "stable").await;
    let first = identity(&app, &guard, &persistent).await;
    let second = identity(&app, &guard, &persistent).await;
    assert_eq!(first, second);

    let ephemeral = camoufox_profile(&app, build_id, ProfileKind::Ephemeral, "fresh").await;
    let mut seen = Vec::new();
    for _ in 0..EPHEMERAL_LAUNCHES {
        let identity = identity(&app, &guard, &ephemeral).await;
        if !seen.contains(&identity) {
            seen.push(identity);
        }
    }
    assert!(seen.len() > 1, "every ephemeral launch drew {:?}", seen[0]);
}

#[tokio::test]
async fn blocks_ads_from_launch() {
    let Some(assets) = assets() else {
        return;
    };
    let ad_hits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&ad_hits);
    let base = serve(
        Router::new()
            .route("/list.txt", get(|| async { "/ad.js$script\n" }))
            .route(
                "/ad.js",
                get(move || async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                    ""
                }),
            )
            .route(
                "/page",
                get(|| async { Html(r#"<script src="/ad.js"></script><p>page</p>"#) }),
            ),
    )
    .await;
    let lists = format!("[\"{base}list.txt\"]");
    let dir = TempDir::new().unwrap();
    let app = app_with_assets(&dir, &assets, &[("BRR_CONTENT__ADBLOCK_LISTS", &lists)]).await;
    let build_id = registered(&app, &assets).await;
    let guard = KillOnDrop::new(&app, Vec::new());
    let mut profile = camoufox_profile(&app, build_id, ProfileKind::Ephemeral, "blocking").await;
    profile.settings.block_ads = true;

    let (instance, tab) = app.instances.launch(&app, request(&profile)).await.unwrap();
    guard.track(instance.id);
    app.instances
        .navigate(
            &instance.id,
            &tab.id,
            format!("{base}page"),
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(ad_hits.load(Ordering::SeqCst), 0);
    app.instances
        .stop(&app, &instance.id, EndReason::Stop)
        .await
        .unwrap();
}

async fn serve(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(std::future::IntoFuture::into_future(axum::serve(
        listener, router,
    )));

    format!("http://{addr}/")
}

async fn call(app: &Arc<App>, name: &str, input: Value) -> Value {
    let op = ops::find(name).unwrap();

    ops::dispatch(ctx(app), op, input).await.unwrap()
}

#[tokio::test]
async fn offline_cookies_bookmarks() {
    let Some(assets) = assets() else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let app = app_with_assets(&dir, &assets, &[]).await;
    let build_id = registered(&app, &assets).await;
    let guard = KillOnDrop::new(&app, Vec::new());
    let url = serve(Router::new().route(
        "/",
        get(|| async {
            (
                [(SET_COOKIE, format!("{COOKIE}; Max-Age=86400; Path=/"))],
                "ok",
            )
        }),
    ))
    .await;
    let found = camoufox_profile(&app, build_id, ProfileKind::Persistent, "jar").await;

    let (instance, tab) = app.instances.launch(&app, request(&found)).await.unwrap();
    guard.track(instance.id);
    app.instances
        .navigate(
            &instance.id,
            &tab.id,
            url,
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await
        .unwrap();
    app.instances
        .stop(&app, &instance.id, EndReason::Stop)
        .await
        .unwrap();

    let exported = call(&app, "profile.cookies_export", json!({"profile": "jar"})).await;
    let data = exported["data"].as_str().unwrap();
    assert!(
        data.contains("\"fox\"") && data.contains("biscuit"),
        "{data}"
    );

    let imported = call(
        &app,
        "profile.bookmarks_import",
        json!({"profile": "jar", "data": BOOKMARKS_HTML}),
    )
    .await;
    let bookmarks = call(&app, "profile.bookmarks_export", json!({"profile": "jar"})).await;
    assert_eq!(imported["imported"], 2);
    let html = bookmarks["data"].as_str().unwrap();
    assert!(html.contains("https://one.example/") && html.contains("https://two.example/"));
    assert_eq!(bookmarks["count"], 2);

    let session = json!({"cookies": [{"name": "s", "value": "1", "domain": "example.com", "path": "/", "expires": -1, "httpOnly": false, "secure": false, "sameSite": "Lax"}], "origins": []});
    let converted = call(
        &app,
        "profile.cookies_import",
        json!({"profile": "jar", "data": session.to_string()}),
    )
    .await;
    assert!(
        converted["note"]
            .as_str()
            .unwrap()
            .starts_with("1 session cookie")
    );
}

#[tokio::test]
async fn download_limit_without_length() {
    let Some(assets) = assets() else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let app = app_with_assets(&dir, &assets, &[]).await;
    let build_id = registered(&app, &assets).await;
    let guard = KillOnDrop::new(&app, Vec::new());
    let found = camoufox_profile(&app, build_id, ProfileKind::Ephemeral, "dl").await;
    // A streamed body has no Content-Length: the size is known only at the end.
    let url = serve(Router::new().route(
        "/",
        get(|| async {
            let chunks = (0..DOWNLOAD_BYTES / DOWNLOAD_CHUNK)
                .map(|_| Ok::<_, Infallible>(Bytes::from(vec![7_u8; DOWNLOAD_CHUNK])));

            (
                [
                    (CONTENT_TYPE, "application/octet-stream"),
                    (CONTENT_DISPOSITION, "attachment; filename=\"big.bin\""),
                ],
                Body::from_stream(futures::stream::iter(chunks)),
            )
        }),
    ))
    .await;
    app.set_settings([("retention.download_max_mb".to_owned(), json!(1))].into())
        .await
        .unwrap();

    let (instance, tab) = app.instances.launch(&app, request(&found)).await.unwrap();
    guard.track(instance.id);
    let instance_id: InstanceId = instance.id;
    // A download aborts the navigation that started it.
    let _ = app
        .instances
        .navigate(
            &instance_id,
            &tab.id,
            url,
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await;

    wait_for("download.cancelled", async || {
        !events_of(&app, "download.cancelled", instance_id)
            .await
            .is_empty()
    })
    .await;
    let cancelled = events_of(&app, "download.cancelled", instance_id).await;
    assert_eq!(cancelled[0]["reason"], "too_large");
    let filter = artifact::Filter {
        kind: Some(brr_core::ArtifactKind::Download),
        ..artifact::Filter::default()
    };
    let (rows, _) = app
        .store
        .run(move |c| artifact::list(c, &filter, &Page::new(None, None)))
        .await
        .unwrap();
    assert!(rows.is_empty());
}

// The Tab's own title follows navigations only; tab.get asks the page.
#[tokio::test]
async fn tab_get_follows_a_title_a_script_sets() {
    let Some(assets) = assets() else {
        return;
    };
    let dir = TempDir::new().unwrap();
    let app = app_with_assets(&dir, &assets, &[]).await;
    let build_id = registered(&app, &assets).await;
    let guard = KillOnDrop::new(&app, Vec::new());
    let found = camoufox_profile(&app, build_id, ProfileKind::Ephemeral, "title").await;
    let (instance, tab) = app.instances.launch(&app, request(&found)).await.unwrap();
    guard.track(instance.id);

    let page = app.instances.page(&instance.id, &tab.id).await.unwrap();
    page.evaluate(
        "setTimeout(() => { document.title = 'later'; }, 300)",
        EVAL_TIMEOUT,
    )
    .await
    .unwrap();

    common::settled(async || {
        let got = call(&app, "tab.get", json!({"tab_id": tab.id})).await;

        got["title"] == "later"
    })
    .await;
    app.instances
        .stop(&app, &instance.id, EndReason::Stop)
        .await
        .unwrap();
}
