#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;

use brr_bitwarden::{Error, Vault};
use brr_core::Secret;
use serde::Deserialize;
use test_case::test_case;

const FIXTURES: &str = "tests/fixtures";
const TOKEN: &str = "token.json";
const SYNC: &str = "sync.json";
const EXPECTED: &str = "expected.json";
const WRONG_PASSWORD_SUFFIX: &str = "-wrong";

#[derive(Deserialize)]
struct Expected {
    master_password: String,
    email: String,
    items: Vec<ExpectedItem>,
    #[serde(default)]
    trashed_ids: Vec<String>,
    #[serde(default)]
    organizations: Vec<ExpectedOrganization>,
}

#[derive(Deserialize)]
struct ExpectedItem {
    id: String,
    name: String,
    uris: Vec<String>,
    username: Option<String>,
    password: Option<String>,
    totp: Option<String>,
}

#[derive(Deserialize)]
struct ExpectedOrganization {
    name: String,
    item_ids: Vec<String>,
}

struct Fixture {
    token: Vec<u8>,
    sync: Vec<u8>,
    expected: Expected,
}

fn fixture(account: &str) -> Option<Fixture> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(FIXTURES)
        .join(account);
    if !dir.is_dir() {
        return None;
    }

    Some(Fixture {
        token: fs::read(dir.join(TOKEN)).unwrap(),
        sync: fs::read(dir.join(SYNC)).unwrap(),
        expected: serde_json::from_slice(&fs::read(dir.join(EXPECTED)).unwrap()).unwrap(),
    })
}

#[test_case("pbkdf2" ; "pbkdf2 account")]
#[test_case("argon2" ; "argon2id account with organization")]
fn fixture_vault_decrypts_to_expected(account: &str) {
    let Some(Fixture {
        token,
        sync,
        expected,
    }) = fixture(account)
    else {
        return;
    };

    let vault = Vault::unlock(
        &token,
        &sync,
        &Secret::new(expected.master_password.as_str()),
    )
    .unwrap();
    assert_eq!(vault.email(), expected.email);

    let items = vault.items();
    let mut ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
    let mut expected_ids: Vec<&str> = expected.items.iter().map(|item| item.id.as_str()).collect();
    ids.sort_unstable();
    expected_ids.sort_unstable();
    assert_eq!(ids, expected_ids);

    for want in &expected.items {
        let item = items.iter().find(|item| item.id == want.id).unwrap();
        let organization = expected
            .organizations
            .iter()
            .find(|org| org.item_ids.contains(&want.id))
            .map(|org| org.name.as_str());
        assert_eq!(item.name, want.name);
        assert_eq!(item.uris, want.uris);
        assert_eq!(item.username, want.username);
        assert_eq!(item.has_totp, want.totp.is_some());
        assert_eq!(item.organization.as_deref(), organization);

        let login = vault.login(&want.id).unwrap();
        assert_eq!(
            login.username.expose(),
            want.username.as_deref().unwrap_or_default()
        );
        assert_eq!(
            login.password.expose(),
            want.password.as_deref().unwrap_or_default()
        );
        assert_eq!(
            login.totp.as_ref().map(Secret::expose),
            want.totp.as_deref()
        );
        assert_eq!(login.uris, want.uris);
    }

    for id in &expected.trashed_ids {
        assert!(matches!(vault.login(id), Err(Error::NotFound(_))));
    }
}

#[test_case("pbkdf2" ; "pbkdf2 account")]
#[test_case("argon2" ; "argon2id account")]
fn fixture_vault_rejects_wrong_password(account: &str) {
    let Some(Fixture {
        token,
        sync,
        expected,
    }) = fixture(account)
    else {
        return;
    };
    let wrong = format!("{}{WRONG_PASSWORD_SUFFIX}", expected.master_password);

    let result = Vault::unlock(&token, &sync, &Secret::new(wrong));

    assert!(matches!(result, Err(Error::WrongPassword)));
}
