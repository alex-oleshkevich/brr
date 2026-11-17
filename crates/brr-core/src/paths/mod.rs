use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use crate::{InstanceId, Settings};

#[cfg(test)]
mod tests;

const APP_DIR: &str = "brr";
const CONFIG_HOME: &str = "XDG_CONFIG_HOME";
const DATA_HOME: &str = "XDG_DATA_HOME";
const STATE_HOME: &str = "XDG_STATE_HOME";
const RUNTIME_DIR: &str = "XDG_RUNTIME_DIR";
const HOME: &str = "HOME";
const DEFAULT_CONFIG_UNDER_HOME: &str = ".config";
const DEFAULT_DATA_UNDER_HOME: &str = ".local/share";
const DEFAULT_STATE_UNDER_HOME: &str = ".local/state";
const FALLBACK_RUNTIME_PARENT: &str = "/tmp";
const CONFIG_FILE: &str = "config.toml";
const DB_FILE: &str = "brr.db";
const SOCKET_FILE: &str = "brr.sock";
const LOCK_FILE: &str = "daemon.lock";
const VAULT_KEY_FILE: &str = "vault.key";
const PROFILES_DIR: &str = "profiles";
const EPHEMERAL_DIR: &str = "ephemeral";
const BUILDS_DIR: &str = "builds";
const STORAGE_DIR: &str = "storage";
const DOWNLOADS_DIR: &str = "downloads";
const RUNS_DIR: &str = "runs";
const LOG_EXTENSION: &str = "log";
const INSTANCE_LOG_DIR: &str = "instances";
const DAEMON_LOG: &str = "daemon.log";
const MCP_LOG: &str = "mcp.log";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub runtime_dir: PathBuf,
}

impl Paths {
    pub fn from_env(var: impl Fn(&str) -> Option<OsString>, uid: u32) -> Paths {
        let home = PathBuf::from(var(HOME).unwrap_or_default());
        let xdg_dir = |name: &str, under_home: &str| {
            let dir = match var(name).filter(|value| !value.is_empty()) {
                Some(dir) => PathBuf::from(dir),
                None => home.join(under_home),
            };

            dir.join(APP_DIR)
        };

        Paths {
            config_dir: xdg_dir(CONFIG_HOME, DEFAULT_CONFIG_UNDER_HOME),
            data_dir: xdg_dir(DATA_HOME, DEFAULT_DATA_UNDER_HOME),
            state_dir: xdg_dir(STATE_HOME, DEFAULT_STATE_UNDER_HOME),
            runtime_dir: match var(RUNTIME_DIR).filter(|value| !value.is_empty()) {
                Some(dir) => PathBuf::from(dir).join(APP_DIR),
                None => Path::new(FALLBACK_RUNTIME_PARENT).join(format!("{APP_DIR}-{uid}")),
            },
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join(CONFIG_FILE)
    }

    pub fn db_file(&self) -> PathBuf {
        self.data_dir.join(DB_FILE)
    }

    pub fn socket(&self, settings: &Settings) -> PathBuf {
        match &settings.server.socket {
            Some(socket) => socket.clone(),
            None => self.runtime_dir.join(SOCKET_FILE),
        }
    }

    pub fn lock_file(&self) -> PathBuf {
        // With the data it protects: two daemons on one data dir conflict whatever their runtime dirs.
        self.data_dir.join(LOCK_FILE)
    }

    pub fn vault_key_file(&self) -> PathBuf {
        self.data_dir.join(VAULT_KEY_FILE)
    }

    pub fn profiles_dir(&self) -> PathBuf {
        self.data_dir.join(PROFILES_DIR)
    }

    pub fn ephemeral_dir(&self) -> PathBuf {
        self.data_dir.join(EPHEMERAL_DIR)
    }

    pub fn builds_dir(&self) -> PathBuf {
        self.data_dir.join(BUILDS_DIR)
    }

    pub fn storage_dir(&self) -> PathBuf {
        self.data_dir.join(STORAGE_DIR)
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.data_dir.join(DOWNLOADS_DIR)
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.data_dir.join(RUNS_DIR)
    }

    pub fn instance_log(&self, instance_id: &InstanceId) -> PathBuf {
        self.state_dir
            .join(INSTANCE_LOG_DIR)
            .join(instance_id.to_string())
            .with_extension(LOG_EXTENSION)
    }

    pub fn daemon_log(&self) -> PathBuf {
        self.state_dir.join(DAEMON_LOG)
    }

    pub fn mcp_log(&self) -> PathBuf {
        self.state_dir.join(MCP_LOG)
    }
}
