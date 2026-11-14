use test_case::test_case;

use super::{CheckStatus, shm_status, userns_status, writable_dir};

const MIB: u64 = 1024 * 1024;

#[test_case(Some("0"), None, None, false, CheckStatus::Fail; "no namespaces")]
#[test_case(None, Some("0\n"), None, false, CheckStatus::Fail; "unprivileged clone off")]
#[test_case(None, None, Some("1"), false, CheckStatus::Warn; "apparmor restricts")]
#[test_case(None, None, None, true, CheckStatus::Warn; "root")]
#[test_case(Some("63000"), Some("1"), Some("0"), false, CheckStatus::Ok; "all fine")]
fn userns(
    max: Option<&str>,
    clone: Option<&str>,
    apparmor: Option<&str>,
    is_root: bool,
    expected: CheckStatus,
) {
    assert_eq!(userns_status(max, clone, apparmor, is_root).0, expected);
}

#[test_case(64 * MIB, CheckStatus::Warn; "docker default")]
#[test_case(2048 * MIB, CheckStatus::Ok; "shm size 2g")]
fn shm(bytes: u64, expected: CheckStatus) {
    assert_eq!(shm_status(bytes).0, expected);
}

#[test]
fn small_shm_names_the_remedy() {
    assert!(shm_status(64 * MIB).1.contains("--shm-size=2g"));
}

#[test]
fn a_missing_dir_is_created_later_not_a_failure() {
    let scratch = tempfile::tempdir().expect("dir");
    let missing = scratch.path().join("brr");

    let (status, detail) = writable_dir(&missing);

    assert_eq!(status, CheckStatus::Warn);
    assert!(detail.contains("created on first start"), "{detail}");
    assert!(!missing.exists(), "the check must not create it");
    assert_eq!(writable_dir(scratch.path()).0, CheckStatus::Ok);
}
