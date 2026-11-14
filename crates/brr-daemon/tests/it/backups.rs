#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{fs, path::PathBuf, sync::Arc};

use brr_core::{
    ArtifactKind, ConflictReason, Error, InstanceState, LoginCheck, LoginCheckId, LoginStatus,
    Order, Page, ProfileId,
};
use brr_daemon::{
    App, ops,
    services::artifacts::{self, ArtifactOwner},
};
use brr_store::{event, instance, login_check};
use serde_json::{Value, json};

use common::{ctx, instance_row, listen, scratch_app};

const PASSPHRASE: &str = "pp-1";
const BACKUP_PASSPHRASE: &str = "Backup-PP-31";
const COOKIES: &[u8] = b"cookie-bytes";
const TEMPLATE: &str = "chromium-headless";
const SITE: &str = "example.com";
const LOOPBACK_IP: &str = "127.0.0.1";
const LOOPBACK_NAME: &str = "localhost";

struct S3Env {
    endpoint: String,
    key: String,
    secret: String,
    bucket: String,
}

fn s3_env() -> Option<S3Env> {
    let get = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    let found = (|| {
        Some(S3Env {
            endpoint: get("BRR_TEST_S3_ENDPOINT")?,
            key: get("BRR_TEST_S3_KEY")?,
            secret: get("BRR_TEST_S3_SECRET")?,
            bucket: get("BRR_TEST_S3_BUCKET")?,
        })
    })();
    if found.is_none() {
        eprintln!(
            "skipped: set BRR_TEST_S3_ENDPOINT, _KEY, _SECRET and _BUCKET to run the live S3 tests"
        );
    }

    found
}

async fn try_call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    ops::dispatch(ctx(app), ops::find(name).unwrap(), input).await
}

async fn call(app: &Arc<App>, name: &str, input: Value) -> Value {
    try_call(app, name, input).await.unwrap()
}

// A Persistent profile with a Cookies file, a Cache dir and one login check.
async fn seeded_profile(app: &Arc<App>, name: &str) -> (ProfileId, PathBuf) {
    let created = call(
        app,
        "profile.create",
        json!({"name": name, "template": TEMPLATE, "tags": ["work"], "notes": "n"}),
    )
    .await;
    let profile_id: ProfileId = serde_json::from_value(created["id"].clone()).unwrap();
    let data_dir = PathBuf::from(created["data_dir"].as_str().unwrap());
    fs::write(data_dir.join("Cookies"), COOKIES).unwrap();
    fs::create_dir_all(data_dir.join("Cache")).unwrap();
    fs::write(data_dir.join("Cache/blob"), b"cache").unwrap();

    let check = LoginCheck {
        id: LoginCheckId::new(),
        profile_id,
        site: SITE.to_string(),
        url: format!("https://{SITE}/me"),
        expect_selector: Some("#me".to_string()),
        expect_text: None,
        credential_id: None,
        auto_relogin: false,
        last_status: LoginStatus::Ok,
        last_checked_at: None,
    };
    app.store
        .run(move |c| login_check::insert(c, &check))
        .await
        .unwrap();

    (profile_id, data_dir)
}

async fn event_kinds(app: &App) -> Vec<String> {
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

async fn enable_backups(app: &Arc<App>) {
    call(
        app,
        "settings.set",
        json!({"backup.passphrase": BACKUP_PASSPHRASE}),
    )
    .await;
}

#[tokio::test]
async fn export_import_round_trip() {
    let (_first_dir, first) = scratch_app().await;
    let (_second_dir, second) = scratch_app().await;
    let (_, data_dir) = seeded_profile(&first, "alpha").await;

    let exported = call(
        &first,
        "profile.export",
        json!({"profile": "alpha", "passphrase": PASSPHRASE, "destination": "download"}),
    )
    .await;
    assert_eq!(exported["filename"], "alpha.tar.zst.age");
    assert!(!exported.to_string().contains(PASSPHRASE));

    let import = |name: Option<&str>| {
        let mut input = json!({"file": exported["file"], "passphrase": PASSPHRASE});
        if let Some(name) = name {
            input["name"] = json!(name);
        }

        call(&second, "profile.import", input)
    };
    let imported = import(None).await;
    let profile = &imported["profile"];
    assert_eq!(profile["name"], "alpha");
    assert_eq!(profile["tags"], json!(["work"]));
    assert_eq!(profile["notes"], "n");
    assert_eq!(profile["kind"], "persistent");
    assert_eq!(imported["missing_extensions"], json!([]));
    let new_dir = PathBuf::from(profile["data_dir"].as_str().unwrap());
    assert_eq!(
        new_dir,
        second
            .paths
            .profiles_dir()
            .join(profile["id"].as_str().unwrap())
    );
    assert_eq!(fs::read(new_dir.join("Cookies")).unwrap(), COOKIES);
    assert!(!new_dir.join("Cache").exists());
    assert!(data_dir.join("Cache").exists());

    let new_id: ProfileId = serde_json::from_value(profile["id"].clone()).unwrap();
    let checks = second
        .store
        .run(move |c| login_check::for_profile(c, &new_id))
        .await
        .unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].site, SITE);
    assert_eq!(checks[0].expect_selector.as_deref(), Some("#me"));
    assert_eq!(checks[0].credential_id, None);

    assert_eq!(import(None).await["profile"]["name"], "alpha-1");
    assert_eq!(import(Some("beta")).await["profile"]["name"], "beta");
    assert!(
        event_kinds(&second)
            .await
            .contains(&"profile.created".to_string())
    );
    assert!(
        second
            .paths
            .data_dir
            .join("tmp")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );
}

#[tokio::test]
async fn wrong_passphrase() {
    let (_first_dir, first) = scratch_app().await;
    let (_second_dir, second) = scratch_app().await;
    seeded_profile(&first, "alpha").await;
    let exported = call(
        &first,
        "profile.export",
        json!({"profile": "alpha", "passphrase": PASSPHRASE}),
    )
    .await;

    let failed = try_call(
        &second,
        "profile.import",
        json!({"file": exported["file"], "passphrase": "wrong"}),
    )
    .await;

    assert!(matches!(
        failed,
        Err(Error::InvalidArgument { field, .. }) if field == "passphrase"
    ));
    let listed = call(&second, "profile.list", json!({})).await;
    assert!(
        listed["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["name"] != "alpha")
    );
    let profiles_dir = second.paths.profiles_dir();
    assert!(!profiles_dir.exists() || profiles_dir.read_dir().unwrap().next().is_none());
    assert!(
        second
            .paths
            .data_dir
            .join("tmp")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );
}

#[tokio::test]
async fn passphrase_gate() {
    let (_dir, app) = scratch_app().await;
    seeded_profile(&app, "alpha").await;
    let update = |schedule: Value| {
        try_call(
            &app,
            "profile.update",
            json!({"profile": "alpha", "backup_schedule": schedule}),
        )
    };

    assert!(matches!(
        update(json!("daily@03:00")).await,
        Err(Error::InvalidArgument { field, message })
            if field == "backup_schedule" && message.contains("settings set backup.passphrase")
    ));

    assert!(matches!(
        try_call(&app, "settings.set", json!({"backup.passphrase": ""})).await,
        Err(Error::InvalidArgument { field, .. }) if field == "backup.passphrase"
    ));
    enable_backups(&app).await;

    assert!(matches!(
        update(json!("hourly")).await,
        Err(Error::InvalidArgument { field, .. }) if field == "backup_schedule"
    ));
    let updated = update(json!("daily@03:00")).await.unwrap();
    assert_eq!(updated["backup_schedule"], "daily@03:00");

    let shown = call(&app, "settings.get", json!({})).await;
    let secret = &shown["secrets"]["backup.passphrase"];
    assert_eq!(secret["set"], true);
    assert_eq!(secret["hint"], Value::Null);
    assert!(!shown.to_string().contains(BACKUP_PASSPHRASE));
    let config = fs::read_to_string(app.paths.config_file()).unwrap();
    assert!(!config.contains(BACKUP_PASSPHRASE));

    let cleared = update(Value::Null).await.unwrap();
    assert_eq!(cleared["backup_schedule"], Value::Null);
}

#[tokio::test]
async fn keep_and_skip() {
    let (_dir, app) = scratch_app().await;
    let (profile_id, _) = seeded_profile(&app, "alpha").await;
    enable_backups(&app).await;
    call(
        &app,
        "profile.update",
        json!({"profile": "alpha", "backup_keep": 2}),
    )
    .await;

    let mut created = Vec::new();
    for _ in 0..3 {
        created.push(call(&app, "backup.create", json!({"profile": "alpha"})).await);
    }

    let listed = call(&app, "backup.list", json!({"profile": "alpha"})).await;
    let ids: Vec<&Value> = listed["backups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| &b["id"])
        .collect();
    assert_eq!(ids, [&created[2]["id"], &created[1]["id"]]);
    let backend = app.storage.backend();
    let object = |backup: &Value| {
        backend
            .local_path(backup["location"].as_str().unwrap())
            .unwrap()
            .unwrap()
    };
    assert!(!object(&created[0]).exists());
    assert!(object(&created[2]).exists());
    assert_eq!(created[2]["backend"], "local");
    assert!(
        created[2]["location"]
            .as_str()
            .unwrap()
            .starts_with(&format!("backups/{profile_id}/"))
    );

    let mut running = instance_row(InstanceState::Running, None);
    running.profile_id = Some(profile_id);
    app.store
        .run(move |c| instance::insert(c, &running))
        .await
        .unwrap();

    let skipped = try_call(&app, "backup.create", json!({"profile": "alpha"})).await;
    assert!(matches!(
        skipped,
        Err(Error::Conflict {
            reason: ConflictReason::ProfileRunning,
            ..
        })
    ));
    let kinds = event_kinds(&app).await;
    assert_eq!(kinds.iter().filter(|k| *k == "backup.completed").count(), 3);
    assert_eq!(kinds.iter().filter(|k| *k == "backup.skipped").count(), 1);

    let deleted = call(
        &app,
        "backup.delete",
        json!({"backup_id": created[2]["id"]}),
    )
    .await;
    assert_eq!(deleted["id"], created[2]["id"]);
    assert!(!object(&created[2]).exists());
}

#[tokio::test]
async fn restore_from_local_backup() {
    let (_first_dir, app) = scratch_app().await;
    seeded_profile(&app, "alpha").await;
    enable_backups(&app).await;
    let backup = call(&app, "backup.create", json!({"profile": "alpha"})).await;

    let restored = call(&app, "backup.restore", json!({"backup_id": backup["id"]})).await;

    assert_eq!(restored["profile"]["name"], "alpha-1");
    let dir = PathBuf::from(restored["profile"]["data_dir"].as_str().unwrap());
    assert_eq!(fs::read(dir.join("Cookies")).unwrap(), COOKIES);
}

#[tokio::test]
async fn s3_backup_restore() {
    let Some(env) = s3_env() else { return };
    let (_dir, app) = scratch_app().await;
    let (profile_id, _) = seeded_profile(&app, "alpha").await;
    enable_backups(&app).await;
    call(
        &app,
        "storage.backend_set",
        json!({
            "backend": "s3",
            "endpoint": env.endpoint,
            "region": "us-east-1",
            "bucket": env.bucket,
            "access_key_id": env.key,
            "path_style": true,
            "secret_access_key": env.secret,
        }),
    )
    .await;

    let backup = call(&app, "backup.create", json!({"profile": "alpha"})).await;
    assert_eq!(backup["backend"], "s3");
    assert!(
        backup["location"]
            .as_str()
            .unwrap()
            .contains(&format!("backups/{profile_id}/"))
    );
    let object = app
        .storage
        .backend()
        .get_bytes(backup["location"].as_str().unwrap(), u64::MAX)
        .await
        .unwrap();
    assert_eq!(object.len() as u64, backup["size_bytes"].as_u64().unwrap());

    let restored = call(&app, "backup.restore", json!({"backup_id": backup["id"]})).await;
    let dir = PathBuf::from(restored["profile"]["data_dir"].as_str().unwrap());
    assert_eq!(fs::read(dir.join("Cookies")).unwrap(), COOKIES);

    call(&app, "backup.delete", json!({"backup_id": backup["id"]})).await;
    assert!(
        app.storage
            .backend()
            .get_bytes(backup["location"].as_str().unwrap(), 1)
            .await
            .is_err()
    );
}

async fn set_s3(app: &Arc<App>, env: &S3Env, endpoint: &str) {
    call(
        app,
        "storage.backend_set",
        json!({
            "backend": "s3",
            "endpoint": endpoint,
            "region": "us-east-1",
            "bucket": env.bucket,
            "access_key_id": env.key,
            "path_style": true,
            "secret_access_key": env.secret,
        }),
    )
    .await;
}

// The same server under another host name is another endpoint: its backend must not be taken for
// the one that holds the objects.
#[tokio::test]
async fn objects_on_another_s3_endpoint_are_left_alone() {
    let Some(env) = s3_env() else { return };
    if !env.endpoint.contains(LOOPBACK_IP) {
        eprintln!(
            "skipped: the endpoint must be on {LOOPBACK_IP} to be reached under another name"
        );
        return;
    }
    let other = env.endpoint.replace(LOOPBACK_IP, LOOPBACK_NAME);
    let (_dir, app) = scratch_app().await;
    seeded_profile(&app, "alpha").await;
    enable_backups(&app).await;
    set_s3(&app, &env, &env.endpoint).await;
    let backup = call(&app, "backup.create", json!({"profile": "alpha"})).await;
    let artifact = artifacts::store_bytes(
        &app,
        ArtifactKind::Screenshot,
        ArtifactOwner::default(),
        "shot.png",
        "image/png",
        b"png".to_vec().into(),
    )
    .await
    .unwrap();
    let backup_location = backup["location"].as_str().unwrap().to_owned();

    set_s3(&app, &env, &other).await;

    let restore = try_call(&app, "backup.restore", json!({"backup_id": backup["id"]})).await;
    assert!(matches!(restore, Err(Error::InvalidArgument { .. })));
    let delete = try_call(&app, "backup.delete", json!({"backup_id": backup["id"]})).await;
    assert!(matches!(delete, Err(Error::InvalidArgument { .. })));
    assert!(matches!(
        artifacts::url(&app, &artifact).await,
        Err(Error::StorageUnavailable { .. })
    ));
    artifacts::delete(&app, &artifact, "test").await.unwrap();

    // Neither delete reached the old bucket's objects.
    set_s3(&app, &env, &env.endpoint).await;
    let backend = app.storage.backend();
    assert!(backend.get_bytes(&backup_location, u64::MAX).await.is_ok());
    assert!(
        backend
            .get_bytes(&artifact.location, u64::MAX)
            .await
            .is_ok()
    );
    call(&app, "backup.delete", json!({"backup_id": backup["id"]})).await;
}

#[tokio::test]
async fn import_validates_the_requested_name() {
    let (_first_dir, first) = scratch_app().await;
    let (_second_dir, second) = scratch_app().await;
    seeded_profile(&first, "alpha").await;
    let exported = call(
        &first,
        "profile.export",
        json!({"profile": "alpha", "passphrase": PASSPHRASE}),
    )
    .await;

    for bad in [
        "has space",
        "",
        &"x".repeat(65),
        &ProfileId::new().to_string(),
    ] {
        let refused = try_call(
            &second,
            "profile.import",
            json!({"file": exported["file"], "passphrase": PASSPHRASE, "name": bad}),
        )
        .await;

        assert!(matches!(
            refused,
            Err(Error::InvalidArgument { field, .. }) if field == "name"
        ));
    }
}

#[tokio::test]
async fn raw_routes_round_trip() {
    let (_first_dir, first) = scratch_app().await;
    let (_second_dir, second) = scratch_app().await;
    let (_, data_dir) = seeded_profile(&first, "alpha").await;
    let (first_addr, second_addr) = (listen(&first).await, listen(&second).await);
    let http = reqwest::Client::new();
    let encoded =
        |text: &str| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, text);

    let downloaded = http
        .post(format!("http://{first_addr}/v1/profiles/alpha/export/raw"))
        .header("X-Brr-Passphrase", encoded(PASSPHRASE))
        .send()
        .await
        .unwrap();
    assert_eq!(downloaded.status(), 200);
    assert_eq!(
        downloaded.headers()["content-disposition"],
        "attachment; filename=\"alpha.tar.zst.age\""
    );
    let archive = downloaded.bytes().await.unwrap();
    assert!(data_dir.join("Cache").exists());

    let import = |passphrase: &str| {
        http.post(format!(
            "http://{second_addr}/v1/profiles/import/raw?name=beta"
        ))
        .header("X-Brr-Passphrase", encoded(passphrase))
        .header("Content-Type", "application/octet-stream")
        .body(archive.clone())
        .send()
    };

    let wrong = import("wrong").await.unwrap();
    assert_eq!(wrong.status(), 400);
    let body: Value = wrong.json().await.unwrap();
    assert_eq!(body["details"]["field"], "passphrase");
    assert!(
        second
            .paths
            .data_dir
            .join("tmp")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );

    let imported: Value = import(PASSPHRASE).await.unwrap().json().await.unwrap();
    assert_eq!(imported["profile"]["name"], "beta");
    let dir = PathBuf::from(imported["profile"]["data_dir"].as_str().unwrap());
    assert_eq!(fs::read(dir.join("Cookies")).unwrap(), COOKIES);
    assert!(!dir.join("Cache").exists());

    let missing = http
        .post(format!("http://{first_addr}/v1/profiles/alpha/export/raw"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 400);
}

#[tokio::test]
async fn profile_delete_keeps_backups_listed_and_restorable() {
    let (_dir, app) = scratch_app().await;
    seeded_profile(&app, "alpha").await;
    enable_backups(&app).await;
    let backup = call(&app, "backup.create", json!({"profile": "alpha"})).await;
    let object = app
        .storage
        .backend()
        .local_path(backup["location"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert!(object.exists());

    call(&app, "profile.delete", json!({"profile": "alpha"})).await;

    assert!(object.exists());
    let listed = call(&app, "backup.list_all", json!({})).await;
    assert_eq!(listed["backups"][0]["id"], backup["id"]);
    assert_eq!(listed["backups"][0]["profile_name"], "alpha");
    assert_eq!(listed["backups"][0]["profile_deleted"], true);
    let restored = call(
        &app,
        "backup.restore",
        json!({"backup_id": backup["id"], "name": "alpha-again"}),
    )
    .await;
    assert_eq!(restored["profile"]["name"], "alpha-again");
}

#[tokio::test]
async fn ephemeral_profile_cannot_be_scheduled() {
    let (_dir, app) = scratch_app().await;
    enable_backups(&app).await;
    call(
        &app,
        "profile.create",
        json!({"name": "temp", "template": TEMPLATE, "kind": "ephemeral"}),
    )
    .await;

    let refused = try_call(
        &app,
        "profile.update",
        json!({"profile": "temp", "backup_schedule": "daily@03:00"}),
    )
    .await;

    assert!(matches!(
        refused,
        Err(Error::InvalidArgument { field, .. }) if field == "backup_schedule"
    ));
}
