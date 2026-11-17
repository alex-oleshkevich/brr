use brr_core::{CredentialId, CredentialSelectors, CredentialSource, Error, ProfileId};
use brr_store::{Connection, Store, credential, credential::CredentialRow};
use tempfile::TempDir;

const NOW: i64 = 1_700_000_000_000;
const ITEM_ID: &str = "0f6c2f0e-6d2c-4a55-9d7e-3a1d7f4b7c11";

const MIGRATIONS_BEFORE_BITWARDEN: [&str; 6] = [
    include_str!("../src/migrations/0001_init.sql"),
    include_str!("../src/migrations/0002_m3.sql"),
    include_str!("../src/migrations/0003_retention_indexes.sql"),
    include_str!("../src/migrations/0004_ux.sql"),
    include_str!("../src/migrations/0005_history.sql"),
    include_str!("../src/migrations/0006_sandbox.sql"),
];

#[allow(clippy::unwrap_used)]
fn open() -> (TempDir, Store) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(&dir.path().join("brr.db")).unwrap();

    (dir, store)
}

fn run<T: Send + 'static>(
    store: &Store,
    f: impl FnOnce(&mut Connection) -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| Error::Internal {
            message: e.to_string(),
        })?
        .block_on(store.run(f))
}

fn local_row() -> CredentialRow {
    CredentialRow {
        id: CredentialId::new(),
        site: "https://example.com".to_string(),
        login_url: None,
        username_enc: Some(b"user-sealed".to_vec()),
        password_enc: Some(b"password-sealed".to_vec()),
        totp_enc: None,
        notes_enc: None,
        selectors: CredentialSelectors::default(),
        created_at: NOW,
        updated_at: NOW,
        source: CredentialSource::Local,
        bitwarden_item_id: None,
    }
}

fn bitwarden_row() -> CredentialRow {
    CredentialRow {
        password_enc: None,
        source: CredentialSource::Bitwarden,
        bitwarden_item_id: Some(ITEM_ID.to_string()),
        ..local_row()
    }
}

#[test]
fn migration_keeps_rows_links_and_login_checks() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("brr.db");
    let credential_id = CredentialId::new();
    let profile_id = ProfileId::new();

    {
        let conn = rusqlite::Connection::open(&path).expect("open raw db");
        for sql in MIGRATIONS_BEFORE_BITWARDEN {
            conn.execute_batch(sql).expect("old migration");
        }
        conn.pragma_update(None, "user_version", 6)
            .expect("set version");
        conn.execute(
            "INSERT INTO profile (id, name, kind, settings, created_at, updated_at) VALUES (?1, 'p', 'persistent', '{}', ?2, ?2)",
            rusqlite::params![profile_id.to_string(), NOW],
        )
        .expect("profile");
        conn.execute(
            "INSERT INTO credential (id, site, username_enc, password_enc, created_at, updated_at) VALUES (?1, 'https://old.example', x'01', x'02', ?2, ?2)",
            rusqlite::params![credential_id.to_string(), NOW],
        )
        .expect("credential");
        conn.execute(
            "INSERT INTO credential_profile (credential_id, profile_id) VALUES (?1, ?2)",
            rusqlite::params![credential_id.to_string(), profile_id.to_string()],
        )
        .expect("link");
        conn.execute(
            "INSERT INTO login_check (id, profile_id, site, url, credential_id) VALUES ('c1', ?1, 'old.example', 'https://old.example/', ?2)",
            rusqlite::params![profile_id.to_string(), credential_id.to_string()],
        )
        .expect("login check");
    }

    let store = Store::open(&path).expect("migrate");
    let (row, linked, check_credential, foreign_keys) = run(&store, move |c| {
        Ok((
            credential::get(c, &credential_id)?,
            credential::profiles(c, &credential_id)?,
            c.query_row("SELECT credential_id FROM login_check", [], |r| {
                r.get::<_, Option<String>>(0)
            })
            .map_err(|e| Error::Internal {
                message: e.to_string(),
            })?,
            c.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                .map_err(|e| Error::Internal {
                    message: e.to_string(),
                })?,
        ))
    })
    .expect("read migrated rows");

    assert_eq!(row.site, "https://old.example");
    assert_eq!(row.username_enc, Some(vec![1]));
    assert_eq!(row.password_enc, Some(vec![2]));
    assert_eq!(row.source, CredentialSource::Local);
    assert_eq!(row.bitwarden_item_id, None);
    assert_eq!(linked, [profile_id]);
    assert_eq!(check_credential, Some(credential_id.to_string()));
    assert_eq!(foreign_keys, 1);
}

#[test]
fn bitwarden_row_round_trips_and_is_found_by_item() {
    let (_dir, store) = open();
    let row = bitwarden_row();
    let id = row.id;

    let (read, found, missing) = run(&store, move |c| {
        credential::insert(c, &row)?;

        Ok((
            credential::get(c, &id)?,
            credential::find_by_bitwarden_item(c, ITEM_ID)?,
            credential::find_by_bitwarden_item(c, "other")?,
        ))
    })
    .expect("bitwarden row");

    assert_eq!(read.source, CredentialSource::Bitwarden);
    assert_eq!(read.bitwarden_item_id.as_deref(), Some(ITEM_ID));
    assert_eq!(read.password_enc, None);
    assert_eq!(read.username_enc, Some(b"user-sealed".to_vec()));
    assert_eq!(found.map(|found| found.id), Some(id));
    assert!(missing.is_none());
}

#[test]
fn local_row_without_a_password_is_rejected() {
    let (_dir, store) = open();
    let row = CredentialRow {
        password_enc: None,
        ..local_row()
    };

    let result = run(&store, move |c| credential::insert(c, &row));

    assert!(matches!(result, Err(Error::Internal { .. })), "{result:?}");
}

#[test]
fn bitwarden_row_without_an_item_id_is_rejected() {
    let (_dir, store) = open();
    let row = CredentialRow {
        bitwarden_item_id: None,
        ..bitwarden_row()
    };

    let result = run(&store, move |c| credential::insert(c, &row));

    assert!(matches!(result, Err(Error::Internal { .. })), "{result:?}");
}
