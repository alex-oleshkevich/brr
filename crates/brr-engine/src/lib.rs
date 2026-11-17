mod camoufox;
mod chromium;
mod common;
pub mod compat;
mod live;
pub mod page;
pub mod procs;
pub mod sandbox;
mod xvfb;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use brr_camoufox::fpgen::model::Model;
use brr_camoufox::geoip::Geo;
use brr_cdp::{Downstream, Outbound};
use brr_core::{
    Cookie, Display, DisplayEnv, Engine, Error, InstanceId, ProfileSettings, TargetId, WaitUntil,
};
use brr_proto::frames::Stream;
use tokio::sync::mpsc;

pub use camoufox::Camoufox;
pub use camoufox::profile_files::{
    export_bookmarks_offline, import_bookmarks_offline, read_cookies_offline, write_cookies_offline,
};
pub use chromium::fetch::{PausedRequest, RequestRules, Verdict, verdict};
pub use chromium::in_container;
pub use chromium::{Chromium, profile_files};
pub use live::{
    AttachSpec, Attached, Connection, INSTALL_NAMES, Live, LiveTarget, devtools_active_port,
};
pub use xvfb::{DEFAULT_VIRTUAL_SCREEN, Xvfb};

#[cfg(test)]
mod tests;

const PROFILE_FIELD: &str = "profile";
const VISIBLE_DISPLAY: &str = "DISPLAY or WAYLAND_DISPLAY";
const DISPLAY_FIELD: &str = "display";
const SANDBOX_VISIBLE_HELP: &str =
    "visible display is not available in the docker sandbox; use headless or virtual";
const VISIBLE_DISPLAY_HELP: &str = "display visible needs a desktop; start the daemon from a desktop session, or use display headless or virtual";

pub const BROWSER_CLOSED: &str = "browser closed before its first tab attached";

#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub instance_id: InstanceId,
    pub engine: Engine,
    pub executable: PathBuf,
    pub user_data_dir: PathBuf,
    pub log_file: PathBuf,
    pub display: Display,
    pub display_env: DisplayEnv,
    pub settings: ProfileSettings,
    pub start_timeout: Duration,
    pub extensions: Vec<PathBuf>,
    pub proxy_server: Option<String>,
    pub download_dir: PathBuf,
    pub console_buffer: usize,
    pub network_buffer: usize,
    // Some for Engine::Camoufox.
    pub camoufox: Option<CamoufoxSpec>,
    // Some for sandbox mode docker: the browser runs in a container of its own.
    pub sandbox: Option<sandbox::SandboxSpec>,
}

#[derive(Clone)]
pub struct CamoufoxSpec {
    pub fonts_dir: PathBuf,
    pub model: Arc<Model>,
    pub geo: Option<Geo>,
    pub seed: Option<u64>,
    pub addons: Vec<PathBuf>,
}

impl std::fmt::Debug for CamoufoxSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CamoufoxSpec")
            .field("fonts_dir", &self.fonts_dir)
            .field("seed", &self.seed)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadState {
    InProgress,
    Completed,
    Canceled,
}

// What a navigation ended in. A response the browser saves as a file commits no document: the
// page stays where it was and the file arrives through the download events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Navigated {
    Page,
    Download,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverEvent {
    TabCreated {
        target_id: TargetId,
        url: String,
    },
    TabAttached {
        target_id: TargetId,
    },
    TabLoading {
        target_id: TargetId,
    },
    TabLoaded {
        target_id: TargetId,
    },
    TabInfo {
        target_id: TargetId,
        url: String,
        title: String,
    },
    TabCrashed {
        target_id: TargetId,
    },
    TabDestroyed {
        target_id: TargetId,
    },
    DownloadStarted {
        target_id: Option<TargetId>,
        guid: String,
        url: String,
        filename: String,
    },
    DownloadProgress {
        guid: String,
        received_bytes: u64,
        total_bytes: Option<u64>,
        state: DownloadState,
    },
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

// `events` is bounded: drain it in its own task and never await a driver call inside that
// loop, or the driver's event task blocks on a full queue and the call never returns.
pub struct Launched {
    pub driver: Driver,
    pub events: mpsc::Receiver<DriverEvent>,
    pub pid: u32,
    pub browser_version: String,
    pub first_target_id: TargetId,
    pub xvfb_pid: Option<u32>,
    // A sandboxed browser's proxy connections, each for the run's forwarder; ends with the run.
    pub tunnels: Option<mpsc::Receiver<Stream>>,
}

#[derive(Clone)]
pub enum Driver {
    Chromium(Chromium),
    Camoufox(Camoufox),
    Live(Live),
}

impl Driver {
    pub async fn launch(mut spec: LaunchSpec) -> Result<Launched, Error> {
        if spec.sandbox.is_some() {
            if spec.display == Display::Visible {
                return Err(Error::InvalidArgument {
                    field: DISPLAY_FIELD.to_owned(),
                    message: SANDBOX_VISIBLE_HELP.to_owned(),
                });
            }
            // The runner's relay is the browser's only way out of its container.
            spec.proxy_server = Some(format!(
                "http://{}:{}",
                sandbox::RELAY_HOST,
                sandbox::RELAY_PORT
            ));
            spec.executable = sandbox::resolve_executable(&spec.executable)?;
        }

        if spec.display == Display::Visible
            && spec.display_env.display.is_none()
            && spec.display_env.wayland_display.is_none()
        {
            return Err(Error::DependencyMissing {
                name: VISIBLE_DISPLAY.to_owned(),
                help: VISIBLE_DISPLAY_HELP.to_owned(),
            });
        }

        let docker_url = spec
            .sandbox
            .as_ref()
            .and_then(|sandbox| sandbox.docker.docker_url.clone());
        let launched = match spec.engine {
            Engine::Chromium => Chromium::launch(spec).await,
            Engine::Camoufox => Camoufox::launch(spec).await,
        };
        // The image or a build volume may have gone from the Host since it was checked.
        if launched.is_err()
            && let Some(url) = &docker_url
        {
            sandbox::forget_host(url);
        }

        launched
    }

    pub async fn open_tab(&self, url: &str) -> Result<TargetId, Error> {
        match self {
            Driver::Chromium(chromium) => chromium.open_tab(url).await,
            Driver::Live(live) => live.open_tab(url).await,
            Driver::Camoufox(camoufox) => camoufox.open_tab(url).await,
        }
    }

    pub async fn close_tab(&self, target_id: &TargetId) -> Result<(), Error> {
        match self {
            Driver::Chromium(chromium) | Driver::Live(Live { chromium, .. }) => {
                chromium.close_tab(target_id).await
            }
            Driver::Camoufox(camoufox) => camoufox.close_tab(target_id).await,
        }
    }

    pub async fn activate_tab(&self, target_id: &TargetId) -> Result<(), Error> {
        match self {
            Driver::Chromium(chromium) => chromium.activate_tab(target_id).await,
            Driver::Live(live) => live.activate_tab(target_id).await,
            Driver::Camoufox(camoufox) => camoufox.activate_tab(target_id).await,
        }
    }

    pub async fn navigate(
        &self,
        target_id: &TargetId,
        url: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<Navigated, Error> {
        match self {
            Driver::Chromium(chromium) => {
                chromium.navigate(target_id, url, wait_until, timeout).await
            }
            Driver::Camoufox(camoufox) => {
                camoufox.navigate(target_id, url, wait_until, timeout).await
            }
            Driver::Live(live) => live.navigate(target_id, url, wait_until, timeout).await,
        }
    }

    pub async fn create_context(&self) -> Result<ContextId, Error> {
        match self {
            Driver::Chromium(chromium) | Driver::Live(Live { chromium, .. }) => {
                chromium.create_context().await
            }
            Driver::Camoufox(camoufox) => camoufox.create_context().await,
        }
    }

    pub async fn open_tab_in(&self, context_id: &ContextId, url: &str) -> Result<TargetId, Error> {
        match self {
            Driver::Chromium(chromium) | Driver::Live(Live { chromium, .. }) => {
                chromium.open_tab_in(context_id, url).await
            }
            Driver::Camoufox(camoufox) => camoufox.open_tab_in(context_id, url).await,
        }
    }

    pub async fn dispose_context(&self, context_id: &ContextId) -> Result<(), Error> {
        match self {
            Driver::Chromium(chromium) | Driver::Live(Live { chromium, .. }) => {
                chromium.dispose_context(context_id).await
            }
            Driver::Camoufox(camoufox) => camoufox.dispose_context(context_id).await,
        }
    }

    pub async fn cookies(&self) -> Result<Vec<Cookie>, Error> {
        match self {
            Driver::Chromium(chromium) | Driver::Live(Live { chromium, .. }) => {
                chromium.cookies().await
            }
            Driver::Camoufox(camoufox) => camoufox.cookies().await,
        }
    }

    pub async fn set_cookies(&self, cookies: &[Cookie]) -> Result<u32, Error> {
        match self {
            Driver::Chromium(chromium) | Driver::Live(Live { chromium, .. }) => {
                chromium.set_cookies(cookies).await
            }
            Driver::Camoufox(camoufox) => camoufox.set_cookies(cookies).await,
        }
    }

    pub async fn cancel_download(&self, guid: &str) -> Result<(), Error> {
        match self {
            Driver::Chromium(chromium) | Driver::Live(Live { chromium, .. }) => {
                chromium.cancel_download(guid).await
            }
            Driver::Camoufox(camoufox) => camoufox.cancel_download(guid).await,
        }
    }

    pub async fn downstream(&self) -> Result<(Downstream, mpsc::Receiver<Outbound>), Error> {
        match self {
            Driver::Chromium(chromium) | Driver::Live(Live { chromium, .. }) => {
                chromium.downstream().await
            }
            Driver::Camoufox(_) => Err(Error::InvalidArgument {
                field: PROFILE_FIELD.to_owned(),
                message: "Camoufox has no /cdp endpoint (ADR 0003)".to_owned(),
            }),
        }
    }

    pub async fn close(&self) -> Result<(), Error> {
        match self {
            Driver::Chromium(chromium) => chromium.close().await,
            Driver::Camoufox(camoufox) => camoufox.close().await,
            Driver::Live(_) => Err(common::internal(
                "brr never closes a Live browser; use instance.detach",
            )),
        }
    }
}
