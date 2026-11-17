use std::{collections::HashMap, ffi::OsString, path::PathBuf};

use crate::{InstanceId, Paths, Settings};

const UID: u32 = 1000;

fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
    let vars: HashMap<String, OsString> = vars
        .iter()
        .map(|(name, value)| (name.to_string(), OsString::from(value)))
        .collect();

    move |name| vars.get(name).cloned()
}

#[test]
fn xdg_vars_give_brr_dirs() {
    let paths = Paths::from_env(
        lookup(&[
            ("XDG_CONFIG_HOME", "/c"),
            ("XDG_DATA_HOME", "/d"),
            ("XDG_STATE_HOME", "/s"),
            ("XDG_RUNTIME_DIR", "/r"),
        ]),
        UID,
    );

    assert_eq!(paths.config_file(), PathBuf::from("/c/brr/config.toml"));
    assert_eq!(paths.db_file(), PathBuf::from("/d/brr/brr.db"));
    assert_eq!(paths.daemon_log(), PathBuf::from("/s/brr/daemon.log"));
    assert_eq!(paths.lock_file(), PathBuf::from("/d/brr/daemon.lock"));
}

#[test]
fn unset_or_empty_vars_fall_back_to_home_and_tmp() {
    let paths = Paths::from_env(lookup(&[("HOME", "/h"), ("XDG_DATA_HOME", "")]), UID);

    assert_eq!(paths.config_dir, PathBuf::from("/h/.config/brr"));
    assert_eq!(paths.data_dir, PathBuf::from("/h/.local/share/brr"));
    assert_eq!(paths.state_dir, PathBuf::from("/h/.local/state/brr"));
    assert_eq!(paths.runtime_dir, PathBuf::from("/tmp/brr-1000"));
}

#[test]
fn socket_prefers_the_setting() {
    let paths = Paths::from_env(lookup(&[("XDG_RUNTIME_DIR", "/r")]), UID);
    let mut settings = Settings::default();

    assert_eq!(paths.socket(&settings), PathBuf::from("/r/brr/brr.sock"));

    settings.server.socket = Some(PathBuf::from("/x/custom.sock"));

    assert_eq!(paths.socket(&settings), PathBuf::from("/x/custom.sock"));
}

#[test]
fn instance_log_is_named_by_id() {
    let paths = Paths::from_env(lookup(&[("XDG_STATE_HOME", "/s")]), UID);
    let instance_id = InstanceId::new();

    assert_eq!(
        paths.instance_log(&instance_id),
        PathBuf::from(format!("/s/brr/instances/{instance_id}.log"))
    );
}

#[test]
fn storage_and_downloads_live_under_data() {
    let paths = Paths::from_env(lookup(&[("XDG_DATA_HOME", "/d")]), UID);

    assert_eq!(paths.storage_dir(), PathBuf::from("/d/brr/storage"));
    assert_eq!(paths.downloads_dir(), PathBuf::from("/d/brr/downloads"));
    assert_eq!(paths.runs_dir(), PathBuf::from("/d/brr/runs"));
}
