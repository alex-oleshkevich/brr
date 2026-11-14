use brr_core::{CredentialId, CredentialSelectors, KeySource, Secret};
use brr_store::Store;
use brr_vault::KeyLocation;

use super::{NewCredential, PASSWORD, new_row, open_field};
use crate::services::secrets::Secrets;

const PASSWORD_VALUE: &str = "S3cret-1";

fn new_credential(username: &str) -> NewCredential {
    NewCredential {
        site: "https://example.com".to_string(),
        login_url: None,
        username: username.to_string(),
        password: Secret::new(PASSWORD_VALUE),
        totp: None,
        notes: None,
        selectors: CredentialSelectors::default(),
    }
}

#[tokio::test]
async fn aad_binds_row() {
    let dir = tempfile::tempdir().unwrap();
    let at = KeyLocation {
        file: dir.path().join("vault.key"),
        keyring_user: "test-user".to_string(),
    };
    let store = Store::open_in_memory().unwrap();
    let secrets = Secrets::bootstrap(&store, &at, Some(KeySource::File))
        .await
        .unwrap();
    let first = new_row(&secrets, CredentialId::new(), &new_credential("u1")).unwrap();
    let second = new_row(&secrets, CredentialId::new(), &new_credential("u2")).unwrap();

    let own = open_field(&secrets, &first.id, PASSWORD, &first.password_enc).unwrap();
    let copied = open_field(&secrets, &second.id, PASSWORD, &first.password_enc);

    assert_eq!(own.expose(), PASSWORD_VALUE);
    assert!(copied.is_err());
}
