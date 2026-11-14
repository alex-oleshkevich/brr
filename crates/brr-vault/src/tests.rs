use std::os::unix::fs::PermissionsExt;

use brr_core::{Error, KeySource};

use crate::{KEY_FILE_DEPENDENCY, KEY_FILE_MODE, KEY_LEN, KeyLocation, Vault};

const NAME: &[u8] = b"name";
const OTHER_NAME: &[u8] = b"other";
const SECRET: &[u8] = b"secret";
const KEY_FILE_NAME: &str = "vault.key";
const KEYRING_USER: &str = "test-user";

fn location(dir: &tempfile::TempDir) -> KeyLocation {
    KeyLocation {
        file: dir.path().join(KEY_FILE_NAME),
        keyring_user: KEYRING_USER.into(),
    }
}

#[test]
fn create_file_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let at = location(&dir);

    Vault::create(KeySource::File, &at).expect("create");

    let meta = std::fs::metadata(&at.file).expect("key file exists");
    assert_eq!(meta.permissions().mode() & 0o777, KEY_FILE_MODE);
    assert_eq!(meta.len(), KEY_LEN as u64);

    let before = std::fs::read(&at.file).expect("read key");
    let err = Vault::create(KeySource::File, &at).expect_err("second create must refuse");
    assert!(matches!(err, Error::Internal { .. }));
    assert_eq!(std::fs::read(&at.file).expect("read key"), before);
}

#[test]
fn round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let at = location(&dir);
    let vault = Vault::create(KeySource::File, &at).expect("create");

    let sealed = vault.encrypt(NAME, SECRET).expect("encrypt");
    assert_eq!(
        vault.decrypt(NAME, &sealed).expect("decrypt").as_slice(),
        SECRET
    );
    assert!(vault.decrypt(OTHER_NAME, &sealed).is_err());
    assert_ne!(vault.encrypt(NAME, SECRET).expect("encrypt"), sealed);
}

#[test]
fn open_reads_created_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let at = location(&dir);
    let sealed = Vault::create(KeySource::File, &at)
        .expect("create")
        .encrypt(NAME, SECRET)
        .expect("encrypt");

    let reopened = Vault::open(KeySource::File, &at).expect("open");
    assert_eq!(
        reopened.decrypt(NAME, &sealed).expect("decrypt").as_slice(),
        SECRET
    );
}

#[test]
fn open_never_creates() {
    let dir = tempfile::tempdir().expect("tempdir");
    let at = location(&dir);

    let err = Vault::open(KeySource::File, &at).expect_err("no key file");
    assert!(
        matches!(err, Error::DependencyMissing { ref name, .. } if name == KEY_FILE_DEPENDENCY)
    );
    assert!(!at.file.exists());
}

#[test]
fn debug_redacts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let at = location(&dir);
    let vault = Vault::create(KeySource::File, &at).expect("create");

    assert_eq!(format!("{vault:?}"), "Vault");
}

#[test]
fn create_ignores_a_stale_temp_file_and_leaves_none_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let at = location(&dir);
    let mut temp = at.file.as_os_str().to_owned();
    temp.push(crate::KEY_TEMP_SUFFIX);
    std::fs::write(&temp, b"short").expect("stale temp");

    Vault::create(KeySource::File, &at).expect("create");

    assert_eq!(
        std::fs::metadata(&at.file).expect("key").len(),
        KEY_LEN as u64
    );
    assert!(!std::path::Path::new(&temp).exists());
}

#[test]
fn open_names_a_damaged_key_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let at = location(&dir);
    std::fs::write(&at.file, b"short").expect("damaged key");

    let err = Vault::open(KeySource::File, &at).expect_err("damaged key must fail");

    let Error::Internal { message } = err else {
        panic!("expected Internal, got {err:?}");
    };
    assert!(
        message.contains(KEY_FILE_NAME) && message.contains(crate::KEY_FILE_HELP),
        "{message}"
    );
}
