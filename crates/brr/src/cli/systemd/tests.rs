use std::{ffi::OsStr, fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

use super::{STOP_GRACE_S, UnitInput, render_unit, unit_path};

fn unit_for(exe: &Path, path_env: &str, xdg: &[(&str, &Path)]) -> String {
    render_unit(&UnitInput {
        exe,
        path_env: OsStr::new(path_env),
        daemon_log: Path::new("/s/daemon.log"),
        stop_timeout_s: 10,
        xdg,
    })
}

#[test]
fn unit_has_the_exec_environment_and_stop_settings() {
    let unit = unit_for(
        Path::new("/opt/brr"),
        "/usr/bin:/x%y",
        &[("XDG_CONFIG_HOME", Path::new("/home/u/.config"))],
    );

    for line in [
        "ExecStart=\"/opt/brr\" daemon run",
        "PassEnvironment=DISPLAY WAYLAND_DISPLAY XDG_RUNTIME_DIR",
        "Environment=\"PATH=/usr/bin:/x%%y\"",
        "Environment=\"XDG_CONFIG_HOME=/home/u/.config\"",
        "StandardOutput=append:/s/daemon.log",
        "StandardError=append:/s/daemon.log",
        "KillMode=mixed",
        &format!("TimeoutStopSec={}", 10 + STOP_GRACE_S),
        "WantedBy=default.target",
    ] {
        assert!(unit.lines().any(|found| found == line), "{line} in\n{unit}");
    }
}

#[test]
fn special_characters_in_the_exe_and_path_are_escaped() {
    let unit = unit_for(Path::new("/My Apps/b%r\\\"$x/brr"), "/a\"b\\c", &[]);

    assert!(
        unit.contains("ExecStart=\"/My Apps/b%%r\\\\\\\"$$x/brr\" daemon run"),
        "{unit}"
    );
    assert!(unit.contains("Environment=\"PATH=/a\\\"b\\\\c\""), "{unit}");
}

// systemd itself is the judge of the quoting; skipped where systemd-analyze is not installed.
#[test]
fn systemd_accepts_an_exe_path_with_a_space() {
    let dir = tempfile::tempdir().expect("dir");
    let exe_dir = dir.path().join("My Apps");
    fs::create_dir(&exe_dir).expect("mkdir");
    let exe = exe_dir.join("brr");
    fs::copy("/bin/true", &exe).expect("copy");
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).expect("chmod");
    let unit_file = dir.path().join("brr.service");
    fs::write(&unit_file, unit_for(&exe, "/usr/bin", &[])).expect("write");

    let Ok(verified) = Command::new("systemd-analyze")
        .args(["--user", "verify"])
        .arg(&unit_file)
        .output()
    else {
        return;
    };

    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stderr)
    );
}

#[test]
fn unit_lives_under_the_user_config_home() {
    assert_eq!(
        unit_path(Path::new("/home/u/.config")),
        Path::new("/home/u/.config/systemd/user/brr.service")
    );
}
