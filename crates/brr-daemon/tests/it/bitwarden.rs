#![allow(clippy::unwrap_used)]

use crate::common;

use std::sync::Arc;

use brr_core::{
    BitwardenStatus, CredentialId, CredentialSelectors, CredentialSource, Error, now_ms,
};
use brr_daemon::{App, ops, services::credentials::unseal};
use brr_store::{credential, secret};
use serde_json::{Value, json};
use test_case::test_case;

use common::{ctx, scratch_app};

const ITEM_ID: &str = "0b2c1d4e-0000-4000-8000-000000000001";
const SITE: &str = "https://example.com";
const DEFAULT_SERVER: &str = "https://vault.bitwarden.com";

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    ops::dispatch(ctx(app), ops::find(name).unwrap(), input).await
}

async fn status(app: &Arc<App>) -> BitwardenStatus {
    serde_json::from_value(call(app, "bitwarden.status", json!({})).await.unwrap()).unwrap()
}

async fn linked_row(app: &Arc<App>) -> CredentialId {
    let now = now_ms();
    let row = credential::CredentialRow {
        id: CredentialId::new(),
        site: SITE.to_owned(),
        login_url: None,
        username_enc: None,
        password_enc: None,
        totp_enc: None,
        notes_enc: None,
        selectors: CredentialSelectors::default(),
        created_at: now,
        updated_at: now,
        source: CredentialSource::Bitwarden,
        bitwarden_item_id: Some(ITEM_ID.to_owned()),
    };
    let id = row.id;
    app.store
        .run(move |c| credential::insert(c, &row))
        .await
        .unwrap();

    id
}

#[tokio::test]
async fn status_when_not_connected() {
    let (_dir, app) = scratch_app().await;

    assert_eq!(
        status(&app).await,
        BitwardenStatus {
            connected: false,
            unlocked: false,
            email: None,
            items: 0,
            synced_at: None,
            error: None,
        }
    );
}

#[test_case("not a url")]
#[test_case("ftp://vault.example")]
#[test_case("http://vault.example")]
#[tokio::test]
async fn connect_to_a_bad_server_stores_nothing(server_url: &str) {
    let (_dir, app) = scratch_app().await;

    let result = call(
        &app,
        "bitwarden.connect",
        json!({
            "server_url": server_url,
            "client_id": "user.1",
            "client_secret": "api-secret-1",
            "master_password": "master-1",
        }),
    )
    .await;

    assert!(
        matches!(&result, Err(Error::InvalidArgument { field, .. }) if field == "server_url"),
        "{result:?}"
    );
    let names = app.store.run(|c| secret::names(c)).await.unwrap();
    assert!(
        names.iter().all(|name| !name.starts_with("bitwarden.")),
        "{names:?}"
    );
    assert_eq!(app.settings().bitwarden.server_url, DEFAULT_SERVER);
    assert!(!status(&app).await.connected);
}

#[tokio::test]
async fn login_while_locked_is_bitwarden_locked() {
    let (_dir, app) = scratch_app().await;
    let id = linked_row(&app).await;

    let result = unseal(&app, &id).await;

    assert!(matches!(result, Err(Error::BitwardenLocked)), "{result:?}");
}

#[tokio::test]
async fn link_while_locked_is_refused() {
    let (_dir, app) = scratch_app().await;

    let result = call(&app, "bitwarden.link", json!({"item_id": "unknown"})).await;

    assert!(matches!(result, Err(Error::BitwardenLocked)), "{result:?}");
    let rows = app.store.run(|c| credential::all(c)).await.unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn linked_credential_lists_without_secrets_and_deletes() {
    let (_dir, app) = scratch_app().await;
    let id = linked_row(&app).await;

    let listed = call(&app, "credential.list", json!({})).await.unwrap();
    let refused = call(
        &app,
        "credential.update",
        json!({"credential_id": id, "password": "pw-2"}),
    )
    .await;
    let deleted = call(&app, "credential.delete", json!({"credential_id": id})).await;

    let item = &listed["credentials"][0];
    assert_eq!(item["source"], "bitwarden");
    assert_eq!(item["bitwarden_item_id"], ITEM_ID);
    assert_eq!(item["username"], "");
    assert!(
        matches!(&refused, Err(Error::InvalidArgument { field, .. }) if field == "password"),
        "{refused:?}"
    );
    assert_eq!(deleted.unwrap()["deleted"], true);
}
