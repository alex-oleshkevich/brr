#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;

use brr_core::Error;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::merge;

fn preferences(dir: &TempDir, content: &str) -> std::path::PathBuf {
    let path = dir.path().join("Default").join("Preferences");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();

    path
}

#[test]
fn merge_sets_dotted_keys() {
    let dir = TempDir::new().unwrap();
    let path = preferences(&dir, r#"{"a":{"b":1}}"#);
    let prefs = BTreeMap::from([
        ("a.c".to_owned(), json!(true)),
        ("intl.accept_languages".to_owned(), json!("de")),
    ]);

    merge(dir.path(), &prefs).unwrap();

    let merged: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        merged,
        json!({"a": {"b": 1, "c": true}, "intl": {"accept_languages": "de"}})
    );
}

#[test]
fn merge_creates_missing_file() {
    let dir = TempDir::new().unwrap();
    let prefs = BTreeMap::from([("x.y".to_owned(), json!(1))]);

    merge(dir.path(), &prefs).unwrap();

    let merged: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("Default/Preferences")).unwrap())
            .unwrap();
    assert_eq!(merged, json!({"x": {"y": 1}}));
}

#[test]
fn merge_refuses_corrupt_file() {
    let dir = TempDir::new().unwrap();
    let path = preferences(&dir, "{not json");
    let prefs = BTreeMap::from([("a".to_owned(), json!(1))]);

    let error = merge(dir.path(), &prefs).unwrap_err();

    assert!(matches!(&error, Error::Internal { message } if message.contains("Preferences")));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "{not json");
}
