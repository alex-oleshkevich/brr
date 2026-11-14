use std::fs;

use brr_core::Paths;
use tempfile::tempdir;

use super::{SweepItem, SweepRule, clear_stale, cutoff_of};

#[test]
fn rules_serialize_as_ui_names() {
    assert_eq!(
        serde_json::to_value(SweepRule::ArtifactsAge).unwrap(),
        "artifacts_age"
    );
    assert_eq!(
        serde_json::to_value(SweepRule::CaptchaSolvesAge).unwrap(),
        "captcha_solves_age"
    );
}

#[test]
fn cutoff_round_trips_through_the_item_id() {
    let item = SweepItem {
        kind: "events".to_owned(),
        id: "before:1234".to_owned(),
        label: String::new(),
        bytes: 0,
    };

    assert_eq!(cutoff_of(&item).unwrap(), 1234);
}

#[test]
fn clear_stale_removes_only_what_an_earlier_run_left() {
    let dir = tempdir().unwrap();
    let paths = Paths {
        config_dir: dir.path().join("config"),
        data_dir: dir.path().join("data"),
        state_dir: dir.path().join("state"),
        runtime_dir: dir.path().join("run"),
    };
    let scratch = paths.data_dir.join("tmp/export-1");
    let staging = paths.data_dir.join("extensions/01ABC.tmp");
    let installed = paths.data_dir.join("extensions/01ABD/1.0");
    let partial = paths
        .storage_dir()
        .join("artifacts/a/.brr-tmp-f.png.01ABC.tmp");
    let stored = paths.storage_dir().join("artifacts/a/f.png");
    for dir in [&scratch, &staging, &installed] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::create_dir_all(stored.parent().unwrap()).unwrap();
    fs::write(&partial, b"half").unwrap();
    fs::write(&stored, b"whole").unwrap();

    clear_stale(&paths);

    assert!(!scratch.exists() && !staging.exists() && !partial.exists());
    assert!(installed.exists() && stored.exists());
}
