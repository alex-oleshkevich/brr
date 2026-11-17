#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{
    fs,
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{Router, http::header::SET_COOKIE, response::Html, routing::get};
use brr_core::{
    AdmissionMode, ConflictReason, Error, InstanceState, Profile, ProfileId, ProfileKind,
    ProfileSettings, WaitUntil, now_ms,
};
use brr_daemon::{
    App, ops,
    services::{cookies, instances::LaunchRequest},
};
use brr_store::{Connection, instance, profile};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use common::{
    KillOnDrop, add_profile, chrome_app, ctx, instance_row, listen, scratch_app, wait_for,
};

const MARKER: &str = "marker-6f1c";
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(30);
const BOOKMARK_URL: &str = "https://fixture.example/";
const BOOKMARKS_FILE: &str = r#"{"roots":{"bookmark_bar":{"children":[{"date_added":"13300000000000000","id":"5","name":"Fixture","type":"url","url":"https://fixture.example/"}],"date_added":"0","id":"1","name":"Bookmarks bar","type":"folder"},"other":{"children":[],"id":"2","name":"Other bookmarks","type":"folder"},"synced":{"children":[],"id":"3","name":"Mobile bookmarks","type":"folder"}},"version":1}"#;

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    let op = ops::find(name).unwrap();

    ops::dispatch(ctx(app), op, input).await
}

fn fixture_cookies(dir: &Path) {
    let default = dir.join("Default");
    fs::create_dir_all(&default).unwrap();
    let connection = Connection::open(default.join("Cookies")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE meta (key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
             INSERT INTO meta VALUES ('version', '24');
             CREATE TABLE cookies (
                creation_utc INTEGER NOT NULL, host_key TEXT NOT NULL, top_frame_site_key TEXT NOT NULL,
                name TEXT NOT NULL, value TEXT NOT NULL, encrypted_value BLOB NOT NULL, path TEXT NOT NULL,
                expires_utc INTEGER NOT NULL, is_secure INTEGER NOT NULL, is_httponly INTEGER NOT NULL,
                last_access_utc INTEGER NOT NULL, has_expires INTEGER NOT NULL, is_persistent INTEGER NOT NULL,
                priority INTEGER NOT NULL, samesite INTEGER NOT NULL, source_scheme INTEGER NOT NULL,
                source_port INTEGER NOT NULL, last_update_utc INTEGER NOT NULL
             );
             INSERT INTO cookies VALUES (1, '.fixture.example', '', 'fixture', 'plain-value', X'', '/', 0, 0, 0, 1, 0, 0, 1, -1, 0, -1, 1);",
        )
        .unwrap();
    fs::write(default.join("Bookmarks"), BOOKMARKS_FILE).unwrap();
}

async fn persistent_profile(app: &Arc<App>, name: &str) -> Profile {
    let id = ProfileId::new();
    let data_dir = app.paths.profiles_dir().join(id.to_string());
    fs::create_dir_all(&data_dir).unwrap();
    let now = now_ms();
    let created = Profile {
        id,
        name: name.to_owned(),
        kind: ProfileKind::Persistent,
        template_id: None,
        template_name: None,
        settings: ProfileSettings::default(),
        pinned: false,
        notes: String::new(),
        tags: Vec::new(),
        data_dir: Some(data_dir),
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

fn storage_state(names: &[(&str, &str)]) -> String {
    let cookies: Vec<Value> = names
        .iter()
        .map(|(name, value)| {
            json!({"name": name, "value": value, "domain": "127.0.0.1", "path": "/", "expires": -1,
                   "httpOnly": false, "secure": false, "sameSite": "Lax"})
        })
        .collect();

    json!({"cookies": cookies, "origins": []}).to_string()
}

#[tokio::test]
async fn offline_roundtrip() {
    let (_dir, app) = scratch_app().await;
    let profile = persistent_profile(&app, "stopped").await;
    fixture_cookies(profile.data_dir.as_ref().unwrap());

    let netscape = call(
        &app,
        "profile.cookies_export",
        json!({"profile": "stopped", "format": "netscape"}),
    )
    .await
    .unwrap();
    let bookmarks = call(
        &app,
        "profile.bookmarks_export",
        json!({"profile": "stopped"}),
    )
    .await
    .unwrap();
    let imported = call(
        &app,
        "profile.cookies_import",
        json!({"profile": "stopped", "data": storage_state(&[("a", "1"), ("b", "2")])}),
    )
    .await
    .unwrap();
    let after = call(
        &app,
        "profile.cookies_export",
        json!({"profile": "stopped"}),
    )
    .await
    .unwrap();

    assert!(
        netscape["data"]
            .as_str()
            .unwrap()
            .contains(".fixture.example")
    );
    assert!(netscape["data"].as_str().unwrap().contains("plain-value"));
    assert!(bookmarks["data"].as_str().unwrap().contains(BOOKMARK_URL));
    assert_eq!(bookmarks["count"], 1);
    assert_eq!(imported, json!({"imported": 2}));
    assert_eq!(after["count"], 3);
    let text = after["data"].as_str().unwrap();
    assert!(text.contains("\"a\"") && text.contains("\"b\""));
}

#[tokio::test]
async fn bookmark_import_skips_present_and_refuses_empty_files() {
    let (_dir, app) = scratch_app().await;
    let profile = persistent_profile(&app, "marks").await;
    fixture_cookies(profile.data_dir.as_ref().unwrap());
    let exported = call(
        &app,
        "profile.bookmarks_export",
        json!({"profile": "marks"}),
    )
    .await
    .unwrap();
    let import = |data: Value| {
        call(
            &app,
            "profile.bookmarks_import",
            json!({"profile": "marks", "data": data}),
        )
    };
    let extra = format!(
        "<DL><DT><A HREF=\"{BOOKMARK_URL}\">Fixture</A><DT><A HREF=\"https://new.example/\">New</A></DL>"
    );

    let reimported = import(exported["data"].clone()).await.unwrap();
    let first = import(json!(extra)).await.unwrap();
    let again = import(json!(extra)).await.unwrap();
    let empty = import(json!("<p>not bookmarks</p>")).await;
    let after = call(
        &app,
        "profile.bookmarks_export",
        json!({"profile": "marks"}),
    )
    .await
    .unwrap();

    assert_eq!(reimported, json!({"imported": 0}));
    assert_eq!(first, json!({"imported": 1}));
    assert_eq!(again, json!({"imported": 0}));
    assert!(matches!(
        empty,
        Err(Error::InvalidArgument { field, message }) if field == "data" && message.contains("no bookmarks found")
    ));
    assert_eq!(after["count"], 2);
}

#[tokio::test]
async fn guards() {
    let (_dir, app) = scratch_app().await;
    let profile = persistent_profile(&app, "busy").await;
    fixture_cookies(profile.data_dir.as_ref().unwrap());
    let mut row = instance_row(InstanceState::Running, None);
    row.profile_id = Some(profile.id);
    app.store
        .run(move |c| instance::insert(c, &row))
        .await
        .unwrap();
    let id = ProfileId::new();
    let ephemeral = Profile {
        id,
        name: "scratch".to_owned(),
        kind: ProfileKind::Ephemeral,
        data_dir: None,
        ..profile.clone()
    };
    app.store
        .run(move |c| profile::insert(c, &ephemeral))
        .await
        .unwrap();

    let running = call(
        &app,
        "profile.bookmarks_import",
        json!({"profile": "busy", "data": "<DL></DL>"}),
    )
    .await;
    let stopped_ephemeral = call(
        &app,
        "profile.cookies_export",
        json!({"profile": "scratch"}),
    )
    .await;

    assert_eq!(
        running.err(),
        Some(Error::Conflict {
            reason: ConflictReason::ProfileRunning,
            detail: None
        })
    );
    assert!(matches!(
        stopped_ephemeral,
        Err(Error::InvalidArgument { field, .. }) if field == "profile"
    ));
}

#[tokio::test]
async fn put_cookies_takes_the_format_from_the_query() {
    let (_dir, app) = scratch_app().await;
    let profile = persistent_profile(&app, "rest").await;
    fixture_cookies(profile.data_dir.as_ref().unwrap());
    let addr = listen(&app).await;
    let netscape = "# Netscape HTTP Cookie File\n.rest.example\tTRUE\t/\tFALSE\t0\tq\tv\n";

    let response = reqwest::Client::new()
        .put(format!(
            "http://{addr}/v1/profiles/rest/cookies?format=netscape"
        ))
        .json(&json!({"data": netscape}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"imported": 1})
    );
}

struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);

        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn values_never_logged() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&logs);
    // Global, so the logs of blocking threads are seen too; the marker is unique to this test.
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || Capture(Arc::clone(&sink)))
            .finish(),
    )
    .unwrap();
    let (_dir, app) = scratch_app().await;
    let profile = persistent_profile(&app, "secret").await;
    fixture_cookies(profile.data_dir.as_ref().unwrap());

    call(
        &app,
        "profile.cookies_import",
        json!({"profile": "secret", "data": storage_state(&[("session", MARKER)])}),
    )
    .await
    .unwrap();
    let invalid = call(
        &app,
        "profile.cookies_import",
        json!({"profile": "secret", "data": format!("not cookies {MARKER}")}),
    )
    .await;
    let events = call(&app, "events.list", json!({})).await.unwrap();

    let logged = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert!(!logged.contains(MARKER));
    assert!(!events.to_string().contains(MARKER));
    assert!(!format!("{invalid:?}").contains(MARKER));
}

#[tokio::test]
async fn running_instance() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let router = Router::new()
        .route(
            "/",
            get(|| async {
                (
                    [(SET_COOKIE, "brr=live; Path=/")],
                    Html("<title>set</title>"),
                )
            }),
        )
        .route(
            "/echo",
            get(|| async { Html("<script>document.title = document.cookie</script>") }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let profile = add_profile(&app, build_id, "live", false, Default::default()).await;
    let (running, tab) = app
        .instances
        .launch(
            &app,
            LaunchRequest {
                profile,
                display: None,
                priority: 0,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    app.instances
        .navigate(
            &running.id,
            &tab.id,
            format!("http://{addr}/"),
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await
        .unwrap();

    let exported = call(&app, "profile.cookies_export", json!({"profile": "live"}))
        .await
        .unwrap();
    let imported = call(
        &app,
        "profile.cookies_import",
        json!({"profile": "live", "data": storage_state(&[("brr2", "x")])}),
    )
    .await
    .unwrap();
    app.instances
        .navigate(
            &running.id,
            &tab.id,
            format!("http://{addr}/echo"),
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await
        .unwrap();

    let state: Value = serde_json::from_str(exported["data"].as_str().unwrap()).unwrap();
    assert!(
        state["cookies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "brr" && c["value"] == "live")
    );
    assert_eq!(imported, json!({"imported": 1}));
    wait_for("the echoed cookies in the title", async || {
        let view = app.instances.view(&running.id).unwrap();
        view.tabs
            .iter()
            .any(|t| t.id == tab.id && t.title.contains("brr2=x"))
    })
    .await;
}

#[tokio::test]
async fn target_selection() {
    let (_dir, app) = scratch_app().await;
    let mine = persistent_profile(&app, "mine").await;
    let other = persistent_profile(&app, "other").await;
    fixture_cookies(mine.data_dir.as_ref().unwrap());
    let insert = |state: InstanceState, owner: ProfileId| {
        let mut row = instance_row(state, None);
        row.profile_id = Some(owner);

        let id = row.id;
        (row, id)
    };
    let (foreign, foreign_id) = insert(InstanceState::Running, other.id);
    let (first, _) = insert(InstanceState::Running, mine.id);
    let (second, _) = insert(InstanceState::Running, mine.id);
    let (starting, _) = insert(InstanceState::Starting, mine.id);
    for row in [foreign, first, second] {
        app.store
            .run(move |c| instance::insert(c, &row))
            .await
            .unwrap();
    }

    let several = cookies::select(&app, &mine, None).await.err();
    let wrong_owner = cookies::select(&app, &mine, Some(&foreign_id)).await.err();
    app.store
        .run(move |c| instance::insert(c, &starting))
        .await
        .unwrap();
    let in_flux = cookies::select(&app, &mine, None).await.err();

    assert!(
        matches!(several, Some(Error::InvalidArgument { field, .. }) if field == "instance_id")
    );
    assert!(
        matches!(wrong_owner, Some(Error::InvalidArgument { field, .. }) if field == "instance_id")
    );
    assert_eq!(
        in_flux,
        Some(Error::Conflict {
            reason: ConflictReason::ProfileRunning,
            detail: None
        })
    );
}
