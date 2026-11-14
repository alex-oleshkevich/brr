#![allow(clippy::unwrap_used)]

use crate::common;

use std::sync::Arc;

use brr_core::{
    ConflictReason, Credential, CredentialId, Error, EventData, InstanceId, Profile, TabId,
    TextUpload,
};
use brr_daemon::{App, ops, services::credentials::unseal};
use brr_store::credential;
use serde_json::{Value, json};

use common::{ctx, scratch_app};

const SITE: &str = "https://Example.com/login";
const SITE_ORIGIN: &str = "https://example.com";
const PASSWORD: &str = "S3cret-1";
const TOTP: &str = "JBSWY3DPEHPK3PXP";
const NOTES: &str = "n-77";
const PROFILE_NAME: &str = "p1";
const CHROME_CSV: &str = "name,url,username,password,note\n\
    dup,https://dup.example/login,alice,pw-1,\n\
    new,https://new.example/login,bob,pw-2,hello\n\
    broken,not a url,carol,pw-3,\n";

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    let op = ops::find(name).unwrap();

    ops::dispatch(ctx(app), op, input).await
}

async fn ok<T: serde::de::DeserializeOwned>(app: &Arc<App>, name: &str, input: Value) -> T {
    serde_json::from_value(call(app, name, input).await.unwrap()).unwrap()
}

async fn create_full(app: &Arc<App>) -> Value {
    call(
        app,
        "credential.create",
        json!({"site": SITE, "username": "u1", "password": PASSWORD, "totp": TOTP, "notes": NOTES}),
    )
    .await
    .unwrap()
}

fn invalid_field(result: Result<Value, Error>) -> String {
    match result {
        Err(Error::InvalidArgument { field, .. }) => field,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

#[tokio::test]
async fn create_hides_secrets() {
    let (_dir, app) = scratch_app().await;

    let created = create_full(&app).await;

    assert_eq!(created["site"], SITE_ORIGIN);
    assert_eq!(created["login_url"], Value::Null);
    assert_eq!(created["totp_set"], true);
    let id: CredentialId = serde_json::from_value(created["id"].clone()).unwrap();
    let row = app
        .store
        .run(move |c| credential::get(c, &id))
        .await
        .unwrap();
    let mut bytes = row.username_enc.clone();
    bytes.extend(&row.password_enc);
    bytes.extend(row.totp_enc.unwrap());
    bytes.extend(row.notes_enc.unwrap());
    for secret in [PASSWORD, TOTP, NOTES] {
        assert!(!contains(created.to_string().as_bytes(), secret));
        assert!(!contains(&bytes, secret));
    }
}

#[tokio::test]
async fn update_partial() {
    let (_dir, app) = scratch_app().await;
    let created = create_full(&app).await;
    let id: CredentialId = serde_json::from_value(created["id"].clone()).unwrap();

    let removed: Credential = ok(
        &app,
        "credential.update",
        json!({"credential_id": id, "totp": null}),
    )
    .await;
    let renamed: Credential = ok(
        &app,
        "credential.update",
        json!({"credential_id": id, "username": "u2"}),
    )
    .await;

    assert!(!removed.totp_set);
    assert_eq!(renamed.username, "u2");
    let unsealed = unseal(&app, &id).await.unwrap();
    assert_eq!(unsealed.password.expose(), PASSWORD);
    assert_eq!(unsealed.username.expose(), "u2");
    assert!(unsealed.totp.is_none());
}

#[tokio::test]
async fn import_csv() {
    let (_dir, app) = scratch_app().await;
    let profile: Profile = ok(
        &app,
        "profile.create",
        json!({"name": PROFILE_NAME, "template": "chromium-headless"}),
    )
    .await;
    call(
        &app,
        "credential.create",
        json!({"site": "https://dup.example", "username": "alice", "password": "old"}),
    )
    .await
    .unwrap();

    let result = call(
        &app,
        "credential.import",
        json!({"format": "chrome", "file": TextUpload(CHROME_CSV.to_string()), "profile_ids": [PROFILE_NAME]}),
    )
    .await
    .unwrap();

    assert_eq!(result, json!({"imported": 1, "skipped": 2}));
    let listed = call(&app, "credential.list", json!({})).await.unwrap();
    let imported = listed["credentials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["username"] == "bob")
        .unwrap();
    assert_eq!(imported["profile_ids"], json!([profile.id]));
}

#[tokio::test]
async fn bare_host_becomes_an_https_origin() {
    let (_dir, app) = scratch_app().await;

    for (given, origin) in [
        ("github.com", "https://github.com"),
        ("  Example.com/login ", "https://example.com"),
        ("localhost:3000", "https://localhost:3000"),
        ("http://intranet", "http://intranet"),
    ] {
        let created = call(
            &app,
            "credential.create",
            json!({"site": given, "username": "u", "password": "p"}),
        )
        .await
        .unwrap();

        assert_eq!(created["site"], origin);
    }
}

#[tokio::test]
async fn validation_and_last_login() {
    let (_dir, app) = scratch_app().await;

    let bad_site = call(
        &app,
        "credential.create",
        json!({"site": "ftp://x", "username": "u", "password": "p"}),
    )
    .await;
    let bad_totp = call(
        &app,
        "credential.create",
        json!({"site": SITE, "username": "u", "password": "p", "totp": "not base32!"}),
    )
    .await;
    let created = create_full(&app).await;
    let id: CredentialId = serde_json::from_value(created["id"].clone()).unwrap();
    app.events
        .publish(EventData::LoginPerformed {
            tab_id: TabId::new(),
            instance_id: InstanceId::new(),
            credential_id: id,
            ok: true,
            step: None,
        })
        .await
        .unwrap();

    let listed = call(&app, "credential.list", json!({})).await.unwrap();

    assert_eq!(invalid_field(bad_site), "site");
    assert_eq!(invalid_field(bad_totp), "totp");
    assert_eq!(listed["credentials"][0]["last_login"]["ok"], true);
}

#[tokio::test]
async fn update_login_url_and_repeated_profiles() {
    let (_dir, app) = scratch_app().await;
    let profile: Profile = ok(
        &app,
        "profile.create",
        json!({"name": PROFILE_NAME, "template": "chromium-headless"}),
    )
    .await;
    let created = create_full(&app).await;
    let id: CredentialId = serde_json::from_value(created["id"].clone()).unwrap();

    let set: Credential = ok(
        &app,
        "credential.update",
        json!({"credential_id": id, "login_url": "https://example.com/in", "profile_ids": [PROFILE_NAME, profile.id]}),
    )
    .await;
    let kept: Credential = ok(&app, "credential.update", json!({"credential_id": id})).await;
    let cleared: Credential = ok(
        &app,
        "credential.update",
        json!({"credential_id": id, "login_url": null}),
    )
    .await;

    assert_eq!(set.login_url.as_deref(), Some("https://example.com/in"));
    assert_eq!(set.profile_ids, [profile.id]);
    assert_eq!(kept.login_url, set.login_url);
    assert_eq!(cleared.login_url, None);
}

#[tokio::test]
async fn concurrent_creates_of_one_login_make_one_credential() {
    let (_dir, app) = scratch_app().await;
    let input = json!({"site": SITE, "username": "u1", "password": PASSWORD});

    let (first, second) = tokio::join!(
        call(&app, "credential.create", input.clone()),
        call(&app, "credential.create", input),
    );

    let (created, refused): (Vec<_>, Vec<_>) = [first, second].into_iter().partition(Result::is_ok);
    assert_eq!(created.len(), 1);
    assert!(matches!(
        refused.as_slice(),
        [Err(Error::Conflict {
            reason: ConflictReason::CredentialExists,
            ..
        })]
    ));
    assert_eq!(
        app.store.run(|c| credential::all(c)).await.unwrap().len(),
        1
    );
}
