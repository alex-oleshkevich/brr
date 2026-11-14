use std::{
    ffi::OsStr,
    fs,
    io::ErrorKind,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use brr_core::{Build, BuildState, DisplayEnv, Engine, KeySource, Paths, Settings};
use brr_vault::{KeyLocation, Vault};
use nix::{sys::statvfs::statvfs, unistd::geteuid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::services::{host::BYTES_PER_MB, quota::BYTES_PER_GB};

#[cfg(test)]
mod tests;

const MIN_SHM_BYTES: u64 = 512 * 1024 * 1024;
const KEY_FILE_MODE: u32 = 0o600;
const MODE_BITS: u32 = 0o777;
const SHM_DIR: &str = "/dev/shm";
const MAX_USERNS: &str = "/proc/sys/user/max_user_namespaces";
const USERNS_CLONE: &str = "/proc/sys/kernel/unprivileged_userns_clone";
const APPARMOR_USERNS: &str = "/proc/sys/kernel/apparmor_restrict_unprivileged_userns";
const PROBE_FILE: &str = ".brr-doctor-probe";

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

pub struct DoctorEnv<'a> {
    pub paths: &'a Paths,
    pub settings: &'a Settings,
    pub display_env: &'a DisplayEnv,
    pub path_env: &'a OsStr,
    pub daemon_running: bool,
}

fn check(name: &str, (status, detail): (CheckStatus, String)) -> Check {
    Check {
        name: name.to_owned(),
        status,
        detail,
    }
}

// The keyring, the listen probe and the disk reads block, so they run off the async workers.
pub async fn checks(env: &DoctorEnv<'_>, builds: &[Build]) -> Vec<Check> {
    let paths = env.paths.clone();
    let settings = env.settings.clone();
    let display_env = env.display_env.clone();
    let path_env = env.path_env.to_owned();
    let daemon_running = env.daemon_running;
    let builds = builds.to_vec();

    let ran = tokio::task::spawn_blocking(move || {
        let env = DoctorEnv {
            paths: &paths,
            settings: &settings,
            display_env: &display_env,
            path_env: &path_env,
            daemon_running,
        };

        run_checks(&env, &builds)
    })
    .await;

    ran.unwrap_or_else(|error| vec![check("doctor", (CheckStatus::Fail, error.to_string()))])
}

// Only its own probe file is ever written, and it is removed at once.
fn run_checks(env: &DoctorEnv<'_>, builds: &[Build]) -> Vec<Check> {
    vec![
        check("data_dir", writable_dir(&env.paths.data_dir)),
        check("config_dir", writable_dir(&env.paths.config_dir)),
        check("listen", listen_status(env)),
        check("keyring", keyring_status(env)),
        check(
            "xvfb",
            tool_status(
                env.path_env,
                "Xvfb",
                "needed for display virtual; install xvfb",
            ),
        ),
        check("display", display_status(env.display_env)),
        check("dev_shm", dev_shm_status()),
        check("user_namespaces", namespaces_status()),
        check("chromium_build", chromium_build_status(builds)),
        check("disk_floor", disk_floor_status(env)),
        check("docker", docker_status()),
        check(
            "xz",
            tool_status(
                env.path_env,
                "xz",
                "install xz-utils; needed for .tar.xz/.deb Builds",
            ),
        ),
    ]
}

fn writable_dir(dir: &Path) -> (CheckStatus, String) {
    if !dir.exists() {
        return (
            CheckStatus::Warn,
            format!(
                "{} does not exist yet; it will be created on first start",
                dir.display()
            ),
        );
    }

    let probe = dir.join(PROBE_FILE);
    let written = fs::write(&probe, b"").and_then(|()| fs::remove_file(&probe));

    match written {
        Ok(()) => (CheckStatus::Ok, format!("{} is writable", dir.display())),
        Err(error) => (
            CheckStatus::Fail,
            format!(
                "{} is not writable ({error}); fix its permissions or point the XDG_* variables elsewhere",
                dir.display()
            ),
        ),
    }
}

fn listen_status(env: &DoctorEnv<'_>) -> (CheckStatus, String) {
    if env.daemon_running {
        return (CheckStatus::Ok, "daemon running".to_owned());
    }

    let bind = env.settings.server.bind;
    match TcpListener::bind(bind) {
        Ok(listener) => drop(listener),
        Err(error) if error.kind() == ErrorKind::AddrInUse => {
            return (
                CheckStatus::Fail,
                format!(
                    "port {} in use; stop the other process or change server.bind",
                    bind.port()
                ),
            );
        }
        Err(error) => {
            return (
                CheckStatus::Fail,
                format!("cannot bind {bind} ({error}); change server.bind"),
            );
        }
    }

    // The runtime dir is created at start; only an existing one can be checked.
    let socket = env.paths.socket(env.settings);
    if let Some(parent) = socket.parent().filter(|parent| parent.exists())
        && let (CheckStatus::Fail, detail) = writable_dir(parent)
    {
        return (CheckStatus::Fail, detail);
    }

    (CheckStatus::Ok, format!("port {} is free", bind.port()))
}

fn keyring_status(env: &DoctorEnv<'_>) -> (CheckStatus, String) {
    match env.settings.vault.key_source {
        Some(KeySource::Keyring) => {
            let at = KeyLocation {
                file: env.paths.vault_key_file(),
                keyring_user: env.paths.data_dir.to_string_lossy().into_owned(),
            };

            match Vault::open(KeySource::Keyring, &at) {
                Ok(_) => (CheckStatus::Ok, "keyring available".to_owned()),
                Err(error) => (CheckStatus::Fail, error.to_string()),
            }
        }
        Some(KeySource::File) => key_file_status(&env.paths.vault_key_file()),
        None => (CheckStatus::Warn, "vault not created yet".to_owned()),
    }
}

fn key_file_status(key_file: &Path) -> (CheckStatus, String) {
    match fs::metadata(key_file) {
        Ok(metadata) if metadata.permissions().mode() & MODE_BITS == KEY_FILE_MODE => (
            CheckStatus::Ok,
            "vault key file present, mode 0600".to_owned(),
        ),
        Ok(_) => (
            CheckStatus::Fail,
            format!(
                "{} must have mode 0600; run chmod 600 on it",
                key_file.display()
            ),
        ),
        Err(_) => (
            CheckStatus::Fail,
            format!(
                "vault key file {} is missing; restore it from a backup, a new key cannot open old secrets",
                key_file.display()
            ),
        ),
    }
}

fn tool_status(path_env: &OsStr, tool: &str, remedy: &str) -> (CheckStatus, String) {
    let found: Option<PathBuf> = std::env::split_paths(path_env)
        .map(|dir| dir.join(tool))
        .find(|candidate| candidate.is_file());

    match found {
        Some(path) => (
            CheckStatus::Ok,
            format!("{tool} found at {}", path.display()),
        ),
        None => (CheckStatus::Warn, format!("{tool} not on PATH; {remedy}")),
    }
}

fn display_status(display_env: &DisplayEnv) -> (CheckStatus, String) {
    if display_env.display.is_some() || display_env.wayland_display.is_some() {
        return (CheckStatus::Ok, "a display is available".to_owned());
    }

    (
        CheckStatus::Warn,
        "display visible is unavailable; set DISPLAY or WAYLAND_DISPLAY before starting the daemon"
            .to_owned(),
    )
}

fn dev_shm_status() -> (CheckStatus, String) {
    match statvfs(SHM_DIR) {
        Ok(stat) => shm_status(stat.blocks() * stat.fragment_size()),
        Err(error) => (
            CheckStatus::Warn,
            format!("cannot read {SHM_DIR} ({error}); mount it with at least 512 MB"),
        ),
    }
}

pub fn shm_status(bytes: u64) -> (CheckStatus, String) {
    if bytes >= MIN_SHM_BYTES {
        return (CheckStatus::Ok, format!("{} MB", bytes / BYTES_PER_MB));
    }

    (
        CheckStatus::Warn,
        format!(
            "{} MB; run Docker with --shm-size=2g (brr adds --disable-dev-shm-usage in containers)",
            bytes / BYTES_PER_MB
        ),
    )
}

fn namespaces_status() -> (CheckStatus, String) {
    let read = |file: &str| fs::read_to_string(file).ok();

    userns_status(
        read(MAX_USERNS).as_deref(),
        read(USERNS_CLONE).as_deref(),
        read(APPARMOR_USERNS).as_deref(),
        geteuid().is_root(),
    )
}

pub fn userns_status(
    max: Option<&str>,
    clone: Option<&str>,
    apparmor: Option<&str>,
    is_root: bool,
) -> (CheckStatus, String) {
    let is = |value: Option<&str>, expected: &str| value.is_some_and(|v| v.trim() == expected);

    if is(max, "0") || is(clone, "0") {
        return (
            CheckStatus::Fail,
            "user namespaces are disabled; enable them (sysctl user.max_user_namespaces) or the Chromium sandbox cannot start".to_owned(),
        );
    }
    if is(apparmor, "1") {
        return (
            CheckStatus::Warn,
            "AppArmor restricts unprivileged user namespaces; allow Chromium in its profile or set kernel.apparmor_restrict_unprivileged_userns=0".to_owned(),
        );
    }
    if is_root {
        return (
            CheckStatus::Warn,
            "running as root; Chromium's sandbox needs --no-sandbox as root, run brr as a normal user".to_owned(),
        );
    }

    (CheckStatus::Ok, "user namespaces available".to_owned())
}

fn chromium_build_status(builds: &[Build]) -> (CheckStatus, String) {
    let usable = builds.iter().any(|build| {
        build.engine == Engine::Chromium
            && matches!(build.state, BuildState::Installed | BuildState::Registered)
    });

    if usable {
        return (
            CheckStatus::Ok,
            "a Chromium-family Build is available".to_owned(),
        );
    }

    (
        CheckStatus::Fail,
        "no Chromium-family Build; run brr build detect or brr build install cft".to_owned(),
    )
}

fn disk_floor_status(env: &DoctorEnv<'_>) -> (CheckStatus, String) {
    let floor_gb = env.settings.retention.free_disk_floor_gb;

    // A fresh install has no data dir yet; the disk it will land on is the nearest ancestor's.
    let existing = env
        .paths
        .data_dir
        .ancestors()
        .find(|dir| dir.exists())
        .unwrap_or(Path::new("/"));

    match statvfs(existing) {
        Ok(stat) => {
            let free_gb = stat.blocks_available() * stat.fragment_size() / BYTES_PER_GB;
            if free_gb >= floor_gb {
                (CheckStatus::Ok, format!("{free_gb} GB free"))
            } else {
                (
                    CheckStatus::Fail,
                    format!(
                        "{free_gb} GB free, below the {floor_gb} GB floor; free disk space or lower retention.free_disk_floor_gb"
                    ),
                )
            }
        }
        Err(error) => (
            CheckStatus::Fail,
            format!("cannot read the free disk space of the data dir ({error})"),
        ),
    }
}

fn docker_status() -> (CheckStatus, String) {
    let in_container = brr_engine::in_container();

    let detail = if in_container {
        "running in a container"
    } else {
        "not in a container"
    };

    (CheckStatus::Ok, detail.to_owned())
}
