#![allow(clippy::expect_used)]

use std::process::Command;

use tempfile::TempDir;

const BRR: &str = env!("CARGO_BIN_EXE_brr");
const USAGE_EXIT: i32 = 2;
const XDG_VARS: [&str; 4] = [
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
];

// The check runs before the daemon is needed, so no daemon starts.
#[test]
fn export_without_output_path_is_a_usage_error() {
    let dirs: Vec<TempDir> = XDG_VARS
        .iter()
        .map(|_| tempfile::tempdir().expect("tempdir"))
        .collect();
    let passphrase = dirs[0].path().join("passphrase");
    std::fs::write(&passphrase, "pp").expect("write passphrase");
    let mut command = Command::new(BRR);
    command
        .args(["profile", "export", "pe", "--passphrase"])
        .arg(&passphrase)
        .env("BRR_VAULT__KEY_SOURCE", "file");
    for (var, dir) in XDG_VARS.iter().zip(&dirs) {
        command.env(var, dir.path());
    }

    let output = command.output().expect("spawn brr");

    assert_eq!(output.status.code(), Some(USAGE_EXIT), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("binary output; pass -o <path>"),
        "{output:?}"
    );
}
