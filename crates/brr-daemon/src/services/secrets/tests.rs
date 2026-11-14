use brr_core::{Error, KeySource, Secret, SecretStatus};
use brr_store::{Store, secret};
use brr_vault::KeyLocation;
use tempfile::TempDir;
use test_case::test_case;

use super::{CHECK_NAME, Secrets};

const NAME: &str = "s3.secret_key";
const OTHER_NAME: &str = "captcha.api_key";
const VALUE: &str = "abcd1234";
const HINT: &str = "…1234";

async fn fixture() -> (TempDir, Store, Secrets) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let secrets = Secrets::bootstrap(&store, &location(&dir), Some(KeySource::File))
        .await
        .unwrap();

    (dir, store, secrets)
}

fn location(dir: &TempDir) -> KeyLocation {
    KeyLocation {
        file: dir.path().join("vault.key"),
        keyring_user: "test-user".to_string(),
    }
}

#[tokio::test]
async fn status_shows_the_last_four_chars_and_the_row_is_encrypted() {
    let (_dir, store, secrets) = fixture().await;

    secrets
        .set(&store, NAME, &Secret::new(VALUE))
        .await
        .unwrap();

    let status = secrets.status(&store, NAME).await.unwrap();
    assert_eq!(
        status,
        SecretStatus {
            set: true,
            hint: Some(HINT.to_string())
        }
    );
    let row = store.run(|c| secret::get(c, NAME)).await.unwrap().unwrap();
    assert!(!row.windows(VALUE.len()).any(|w| w == VALUE.as_bytes()));
    assert_eq!(
        secrets.get(&store, NAME).await.unwrap().unwrap().expose(),
        VALUE
    );
}

#[tokio::test]
async fn unset_and_deleted_secrets_are_not_set() {
    let (_dir, store, secrets) = fixture().await;
    let unset = SecretStatus {
        set: false,
        hint: None,
    };

    assert_eq!(secrets.status(&store, NAME).await.unwrap(), unset);

    secrets
        .set(&store, NAME, &Secret::new(VALUE))
        .await
        .unwrap();
    secrets.delete(&store, NAME).await.unwrap();

    assert_eq!(secrets.status(&store, NAME).await.unwrap(), unset);
}

#[tokio::test]
async fn short_values_do_not_leak_through_the_hint() {
    let (_dir, store, secrets) = fixture().await;

    secrets
        .set(&store, NAME, &Secret::new("abc"))
        .await
        .unwrap();

    let status = secrets.status(&store, NAME).await.unwrap();
    assert_eq!(status.hint.as_deref(), Some("…"));
}

#[tokio::test]
async fn a_row_moved_to_another_name_does_not_decrypt() {
    let (_dir, store, secrets) = fixture().await;
    secrets
        .set(&store, NAME, &Secret::new(VALUE))
        .await
        .unwrap();

    let sealed = store.run(|c| secret::get(c, NAME)).await.unwrap().unwrap();
    store
        .run(move |c| secret::put(c, OTHER_NAME, &sealed, 0))
        .await
        .unwrap();

    let error = secrets.get(&store, OTHER_NAME).await.unwrap_err();
    assert!(matches!(error, Error::Internal { .. }), "{error:?}");
}

#[tokio::test]
async fn the_vault_check_name_is_reserved() {
    let (_dir, store, secrets) = fixture().await;

    for result in [
        secrets.set(&store, CHECK_NAME, &Secret::new(VALUE)).await,
        secrets.get(&store, CHECK_NAME).await.map(|_| ()),
        secrets.status(&store, CHECK_NAME).await.map(|_| ()),
        secrets.delete(&store, CHECK_NAME).await,
    ] {
        assert!(matches!(result, Err(Error::InvalidArgument { .. })));
    }
}

#[test_case(None; "no configured source defaults to the file")]
#[test_case(Some(KeySource::File); "file")]
#[tokio::test]
async fn a_new_vault_creates_its_key_file(configured: Option<KeySource>) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let at = location(&dir);

    Secrets::bootstrap(&store, &at, configured).await.unwrap();

    assert!(at.file.exists());
    assert!(
        store
            .run(|c| secret::get(c, CHECK_NAME))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn an_existing_vault_needs_its_configured_source_and_key() {
    let (dir, store, _secrets) = fixture().await;
    let at = location(&dir);

    let lost_config = Secrets::bootstrap(&store, &at, None).await.err().unwrap();
    assert!(
        matches!(lost_config, Error::InvalidArgument { .. }),
        "{lost_config:?}"
    );

    std::fs::remove_file(&at.file).unwrap();
    let lost_key = Secrets::bootstrap(&store, &at, Some(KeySource::File))
        .await
        .err()
        .unwrap();
    assert!(
        matches!(lost_key, Error::DependencyMissing { .. }),
        "{lost_key:?}"
    );
    assert!(!at.file.exists(), "a new key must never be created");
}

#[tokio::test]
async fn a_different_key_fails_the_check() {
    let (dir, store, _secrets) = fixture().await;
    let other_dir = tempfile::tempdir().unwrap();
    let other_store = Store::open_in_memory().unwrap();
    let other = location(&other_dir);
    Secrets::bootstrap(&other_store, &other, Some(KeySource::File))
        .await
        .unwrap();

    std::fs::copy(&other.file, location(&dir).file).unwrap();

    let error = Secrets::bootstrap(&store, &location(&dir), Some(KeySource::File))
        .await
        .err()
        .unwrap();
    assert!(matches!(error, Error::Internal { .. }), "{error:?}");
}

#[tokio::test]
async fn sealed_bytes_open_only_with_their_aad() {
    let (_dir, _store, secrets) = fixture().await;

    let sealed = secrets.seal(NAME, &Secret::new(VALUE)).unwrap();

    assert_eq!(secrets.open(NAME, &sealed).unwrap().expose(), VALUE);
    assert!(secrets.open(OTHER_NAME, &sealed).is_err());
}
