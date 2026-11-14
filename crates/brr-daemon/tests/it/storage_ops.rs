#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{fs, sync::Arc};

use brr_core::{ArtifactKind, Error, ProfileKind, capture::ReturnMode};
use brr_daemon::{
    App, ops,
    services::artifacts::{self, ArtifactOwner},
};
use brr_store::profile;
use serde_json::{Value, json};

use common::{ctx, listen, profile_row, scratch_app};

const MIB: usize = 1024 * 1024;
const WRONG_SECRET: &str = "s3cr3t-XYZ";

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

async fn call(app: &Arc<App>, name: &str, input: Value) -> Value {
    ops::dispatch(ctx(app), ops::find(name).unwrap(), input)
        .await
        .unwrap()
}

#[tokio::test]
async fn backend_get_names_local_root() {
    let (_dir, app) = scratch_app().await;

    let shown = call(&app, "storage.backend_get", json!({})).await;
    assert_eq!(shown["backend"], "local");
    assert_eq!(shown["local_root"], json!(app.paths.storage_dir()));
}

#[tokio::test]
async fn backend_test_fails_fast_with_short_message() {
    let (_dir, app) = scratch_app().await;
    let started = std::time::Instant::now();

    let result = ops::dispatch(
        ctx(&app),
        ops::find("storage.backend_test").unwrap(),
        json!({"backend": "s3", "endpoint": "http://127.0.0.1:1", "bucket": "b",
               "access_key_id": "k", "secret_access_key": "s", "path_style": true}),
    )
    .await;

    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert!(matches!(
        result,
        Err(Error::StorageUnavailable { message })
            if message == "Couldn't reach http://127.0.0.1:1: no response from the host"
    ));
}

#[tokio::test]
async fn usage() {
    let (_dir, app) = scratch_app().await;
    let data_dir = app.paths.profiles_dir().join("usage");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("blob"), vec![0; MIB]).unwrap();
    let persistent = profile_row(ProfileKind::Persistent, Some(data_dir), None);
    let profile_id = persistent.id;
    app.store
        .run(move |c| profile::insert(c, &persistent))
        .await
        .unwrap();
    for (kind, name) in [
        (ArtifactKind::Screenshot, "a.png"),
        (ArtifactKind::Pdf, "a.pdf"),
    ] {
        artifacts::store_bytes(
            &app,
            kind,
            ArtifactOwner::default(),
            name,
            "application/octet-stream",
            vec![1; 10].into(),
        )
        .await
        .unwrap();
    }

    let usage = call(&app, "storage.usage", json!({})).await;

    assert_eq!(usage["profiles"][0]["profile_id"], json!(profile_id));
    assert!(usage["profiles"][0]["bytes"].as_u64().unwrap() >= MIB as u64);
    let kinds: Vec<(&str, u64, u64)> = usage["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["kind"].as_str().unwrap(),
                row["count"].as_u64().unwrap(),
                row["bytes"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(kinds, [("pdf", 1, 10), ("screenshot", 1, 10)]);
    assert!(usage["free_bytes"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn settings_set_refuses_storage_keys() {
    let (_dir, app) = scratch_app().await;

    let refused = ops::dispatch(
        ctx(&app),
        ops::find("settings.set").unwrap(),
        json!({"storage.backend": "s3"}),
    )
    .await;

    assert!(matches!(
        refused,
        Err(Error::InvalidArgument { field, .. }) if field == "storage.backend"
    ));
}

#[tokio::test]
async fn backend_set_and_presign() {
    let Some(env) = s3_env() else { return };
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let http = reqwest::Client::new();
    let put = |secret: &str| {
        http.put(format!("http://{addr}/v1/storage/backend"))
            .json(&json!({
                "backend": "s3",
                "endpoint": env.endpoint,
                "region": "us-east-1",
                "bucket": env.bucket,
                "access_key_id": env.key,
                "path_style": true,
                "secret_access_key": secret,
            }))
    };
    let before = app.settings().storage.clone();

    let wrong = put(WRONG_SECRET).send().await.unwrap();
    assert_eq!(wrong.status(), 503);
    let body = wrong.text().await.unwrap();
    assert!(body.contains("StorageUnavailable"), "{body}");
    assert!(!body.contains(WRONG_SECRET), "{body}");
    assert_eq!(app.settings().storage, before);

    let right = put(&env.secret).send().await.unwrap();
    assert_eq!(right.status(), 200, "{:?}", right.text().await);
    let shown = call(&app, "storage.backend_get", json!({})).await;
    assert_eq!(shown["backend"], "s3");
    assert_eq!(shown["secret_access_key"]["set"], true);
    let tail = &env.secret[env.secret.len() - 4..];
    assert_eq!(shown["secret_access_key"]["hint"], format!("…{tail}"));
    assert!(shown["error"].is_null());
    assert!(!shown.to_string().contains(&env.secret));

    let tested = call(
        &app,
        "storage.backend_test",
        json!({"backend": "s3", "endpoint": env.endpoint, "bucket": env.bucket,
               "access_key_id": env.key, "path_style": true}),
    )
    .await;
    assert_eq!(tested["ok"], true);

    let data = vec![9; 6 * MIB];
    let delivery = artifacts::deliver(
        &app,
        ArtifactKind::Pdf,
        ArtifactOwner::default(),
        "big.pdf",
        "application/pdf",
        data.clone(),
        ReturnMode::Auto,
    )
    .await
    .unwrap();
    let url = delivery.url.unwrap();
    assert!(!url.contains("/v1/a/"), "{url}");
    let fetched = reqwest::get(&url).await.unwrap();
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.bytes().await.unwrap().as_ref(), data.as_slice());

    let artifact_id = delivery.artifact_id.unwrap();
    call(&app, "artifact.delete", json!({"artifact_id": artifact_id})).await;
}
