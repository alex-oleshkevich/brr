use std::{fs, os::unix::fs::symlink};

use tempfile::tempdir;
use test_case::test_case;

use super::{is_plain_guid, staged_file};

const UUID: &str = "3f2b8c1e-9d4a-4b7e-8a61-0c5d2e7f9a10";

#[test_case(UUID, true; "chromium uuid")]
#[test_case("{3f2b8c1e-9d4a-4b7e-8a61-0c5d2e7f9a10}", true; "firefox braces")]
#[test_case("", false; "empty")]
#[test_case("..", false; "parent")]
#[test_case("../secret", false; "traversal")]
#[test_case("/etc/passwd", false; "absolute")]
#[test_case("dir/file", false; "separator")]
#[test_case("a.b", false; "dot")]
fn guid_is_a_plain_file_name(guid: &str, expected: bool) {
    assert_eq!(is_plain_guid(guid), expected);
}

#[tokio::test]
async fn staged_file_must_be_a_real_file_under_the_download_dir() {
    let dir = tempdir().unwrap();
    let downloads = dir.path().join("downloads");
    let secret = dir.path().join("secret");
    fs::create_dir_all(&downloads).unwrap();
    fs::write(downloads.join(UUID), b"download").unwrap();
    fs::write(&secret, b"secret").unwrap();
    symlink(&secret, downloads.join("linked")).unwrap();

    assert_eq!(
        staged_file(&downloads, UUID).await.unwrap(),
        downloads.canonicalize().unwrap().join(UUID)
    );
    assert!(
        staged_file(&downloads, secret.to_str().unwrap())
            .await
            .is_err(),
        "an absolute guid replaces the base path"
    );
    assert!(staged_file(&downloads, "../secret").await.is_err());
    assert!(staged_file(&downloads, "linked").await.is_err());
    assert!(staged_file(&downloads, "missing").await.is_err());
}
