use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use test_case::test_case;

use super::{detect_system, parse_version};

const CHROME_OUTPUT: &str = "Google Chrome 152.0.7444.59 ";
const CHROMIUM_OUTPUT: &str = "Chromium 151.0.7400.1  built on Arch Linux";
const BRAVE_OUTPUT: &str = "Brave Browser 1.84.112 Chromium: 142.0.1";
const SCRIPT_VERSION: &str = "152.0.1.2";

#[test_case(CHROME_OUTPUT, Some("152.0.7444.59"))]
#[test_case(CHROMIUM_OUTPUT, Some("151.0.7400.1"))]
#[test_case(BRAVE_OUTPUT, Some("1.84.112"))]
#[test_case("Chrome 152", None)]
#[test_case("", None)]
fn parse_version_takes_first_dotted_number(output: &str, expected: Option<&str>) {
    assert_eq!(parse_version(output).as_deref(), expected);
}

fn write_script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn detect_dedupes_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("chromium");
    write_script(&target, &format!("echo 'Chromium {SCRIPT_VERSION}'"));
    std::os::unix::fs::symlink(&target, dir.path().join("chromium-browser")).unwrap();
    let canonical = target.canonicalize().unwrap();

    // Chrome's vendor install at /opt may exist on the host and hides the google-chrome wrappers.
    let detected = detect_system(dir.path().as_os_str()).await;
    let ours: Vec<_> = detected
        .iter()
        .filter(|d| d.path.starts_with(dir.path().canonicalize().unwrap()))
        .collect();

    assert_eq!(ours.len(), 1);
    assert_eq!(ours[0].name, "Chromium");
    assert_eq!(ours[0].version, SCRIPT_VERSION);
    assert_eq!(ours[0].path, canonical);
}

#[tokio::test]
async fn detect_follows_table_order_and_skips_unparsable() {
    let dir = tempfile::tempdir().unwrap();
    write_script(
        &dir.path().join("brave-browser"),
        "echo 'Brave Browser 1.84.112'",
    );
    write_script(&dir.path().join("chromium"), "echo 'Chromium 151.0.7400.1'");
    write_script(&dir.path().join("microsoft-edge"), "echo 'no version here'");

    let detected = detect_system(dir.path().as_os_str()).await;
    let names: Vec<_> = detected
        .iter()
        .filter(|d| d.path.starts_with(dir.path().canonicalize().unwrap()))
        .map(|d| d.name.as_str())
        .collect();

    assert_eq!(names, ["Chromium", "Brave"]);
}
