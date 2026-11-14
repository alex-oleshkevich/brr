mod contexts;
pub(crate) mod downloads;
pub(crate) mod fetch;
mod input;
mod live;
mod prefs;
pub mod profile_files;
mod screencast;
mod session;
mod targets;

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use brr_cdp::{Cdp, SessionId};
use brr_core::{Cookie, Display, Error, TargetId, WaitUntil, WebRtcPolicy};
use serde_json::{Value, json};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use self::targets::Mode;
pub(crate) use self::targets::is_user_page;
use crate::common::{
    BLANK_PAGE, CONNECTION_CLOSED, ENV_DAEMON, ENV_DISPLAY, ENV_INSTANCE_ID, ENV_TZ,
    ENV_WAYLAND_DISPLAY, ENV_XAUTHORITY, EVENTS_QUEUE, die_with_parent, internal,
};
use crate::page::{
    ContentOptions, EmulateOptions, Page, PageSession, PageState, Session, cookie_params,
    parse_cookies,
};
use crate::xvfb::{DEFAULT_VIRTUAL_SCREEN, Xvfb};
use crate::{Driver, DriverEvent, LaunchSpec, Launched};

pub(crate) use crate::common::millis;

#[cfg(test)]
mod tests;

const EXTENSIONS_FIELD: &str = "extensions";
const LEADING_FLAGS: [&str; 3] = [
    "--remote-debugging-pipe",
    "--password-store=basic",
    "--enable-unsafe-extension-debugging",
];
const TRAILING_FLAGS: [&str; 5] = [
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
];
const USER_DATA_DIR_FLAG: &str = "--user-data-dir=";
const HEADLESS_FLAG: &str = "--headless";
const DISABLE_DEV_SHM_FLAG: &str = "--disable-dev-shm-usage";

#[derive(Clone)]
pub struct Chromium {
    inner: Arc<Inner>,
}

struct Inner {
    cdp: Cdp,
    download_dir: PathBuf,
    console_buffer: usize,
    network_buffer: usize,
    // Applied to every page that attaches: the Profile's viewport and geolocation.
    profile_view: EmulateOptions,
    // What a Tab without options of its own does to its pages; copied into each new Tab.
    content_defaults: Mutex<Option<ContentOptions>>,
    tabs: Mutex<HashMap<TargetId, Tab>>,
    // Every session whose frame events feed a page: the page's own and its iframe children.
    routes: Mutex<HashMap<SessionId, Arc<PageState>>>,
}

struct Tab {
    session_id: SessionId,
    state: Arc<PageState>,
}

pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".brr-tmp");
    let temp = PathBuf::from(temp);

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(internal)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)
        .map_err(internal)?;
    file.write_all(bytes).map_err(internal)?;
    file.sync_all().map_err(internal)?;

    std::fs::rename(&temp, path).map_err(internal)
}

fn wanted_lifecycle(wait_until: WaitUntil) -> &'static str {
    match wait_until {
        WaitUntil::Load => "load",
        WaitUntil::Domcontentloaded => "DOMContentLoaded",
        WaitUntil::Networkidle => "networkIdle",
    }
}

pub fn in_container() -> bool {
    Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists()
}

pub(crate) fn args(spec: &LaunchSpec, in_container: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = LEADING_FLAGS.iter().map(OsString::from).collect();

    let mut user_data_dir = OsString::from(USER_DATA_DIR_FLAG);
    user_data_dir.push(&spec.user_data_dir);
    args.push(user_data_dir);
    args.extend(TRAILING_FLAGS.iter().map(OsString::from));

    if spec.display == brr_core::Display::Headless {
        args.push(HEADLESS_FLAG.into());
    }
    if let Some(screen) = &spec.settings.screen {
        args.push(format!("--window-size={},{}", screen.width, screen.height).into());
    }
    if let Some(locale) = &spec.settings.locale {
        args.push(format!("--lang={locale}").into());
    }
    if let Some(user_agent) = &spec.settings.user_agent {
        args.push(format!("--user-agent={user_agent}").into());
    }
    if let Some(proxy_server) = &spec.proxy_server {
        args.push(format!("--proxy-server={proxy_server}").into());
        // Chromium bypasses loopback implicitly; the negated entry sends it through the proxy
        // too, so a proxied Profile never reaches the host's local services directly.
        args.push("--proxy-bypass-list=<-loopback>".into());
    }
    let webrtc_policy = match spec.settings.webrtc {
        WebRtcPolicy::Default => None,
        WebRtcPolicy::DefaultPublicAndPrivateInterfaces => {
            Some("default_public_and_private_interfaces")
        }
        WebRtcPolicy::DefaultPublicInterfaceOnly => Some("default_public_interface_only"),
        WebRtcPolicy::DisableNonProxiedUdp => Some("disable_non_proxied_udp"),
    };
    if let Some(policy) = webrtc_policy {
        args.push(format!("--force-webrtc-ip-handling-policy={policy}").into());
    }
    if in_container {
        // A container's /dev/shm is 64 MB by default, too small for Chromium's renderers.
        args.push(DISABLE_DEV_SHM_FLAG.into());
    }
    args.extend(spec.settings.args.iter().map(OsString::from));
    args.push(BLANK_PAGE.into());

    args
}

fn command(spec: &LaunchSpec, xvfb: Option<&Xvfb>) -> Result<Command, Error> {
    if !spec.settings.prefs.is_empty() {
        prefs::merge(&spec.user_data_dir, &spec.settings.prefs)?;
    }
    if let Some(dir) = spec.log_file.parent() {
        std::fs::create_dir_all(dir).map_err(internal)?;
    }
    // A file, never a pipe: an unread full pipe blocks the browser.
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&spec.log_file)
        .map_err(internal)?;
    let log_err = log.try_clone().map_err(internal)?;

    let mut command = Command::new(&spec.executable);
    command.args(args(spec, in_container()));
    if let Some(timezone) = &spec.settings.timezone {
        command.env(ENV_TZ, timezone);
    }
    command
        .envs(&spec.settings.env)
        .env(ENV_INSTANCE_ID, spec.instance_id.to_string())
        .env(ENV_DAEMON, "1")
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err)
        .process_group(0)
        .kill_on_drop(false);
    die_with_parent(&mut command);
    if let Some(xvfb) = xvfb {
        command
            .env(ENV_DISPLAY, xvfb.display())
            .env(ENV_XAUTHORITY, xvfb.auth_file())
            .env_remove(ENV_WAYLAND_DISPLAY);
    }

    Ok(command)
}

impl Chromium {
    pub(crate) async fn launch(mut spec: LaunchSpec) -> Result<Launched, Error> {
        // The window fills the virtual screen, so the page sees the size the Profile asked for.
        let xvfb = if spec.display == Display::Virtual {
            let screen = spec
                .settings
                .screen
                .clone()
                .unwrap_or(DEFAULT_VIRTUAL_SCREEN);
            let xvfb = Xvfb::start(screen.clone(), &spec.instance_id, &spec.log_file).await?;
            spec.settings.screen.get_or_insert(screen);

            Some(xvfb)
        } else {
            None
        };
        let xvfb_pid = xvfb.as_ref().map(Xvfb::pid);
        let spawned = match command(&spec, xvfb.as_ref())
            .and_then(|command| brr_proto::spawn(command).map_err(internal))
        {
            Ok(spawned) => spawned,
            Err(error) => {
                // Stopping removes its auth file; dropping it would only kill the process.
                if let Some(mut xvfb) = xvfb {
                    xvfb.stop().await;
                }

                return Err(error);
            }
        };
        let mut child = spawned.child;
        let pid = child
            .id()
            .ok_or_else(|| internal("browser exited at once"))?;
        let (events_tx, events) = mpsc::channel(EVENTS_QUEUE);

        let exit_tx = events_tx.clone();
        tokio::spawn(async move {
            let (code, signal) = match child.wait().await {
                Ok(status) => (status.code(), status.signal()),
                Err(_) => (None, None),
            };
            if let Some(mut xvfb) = xvfb {
                xvfb.stop().await;
            }
            let _ = exit_tx.send(DriverEvent::Exited { code, signal }).await;
        });

        let (cdp, cdp_events) = Cdp::start(spawned.mux, spawned.events);
        let browser_version = cdp.browser_version().await?;
        cdp.set_discover_targets().await?;
        cdp.set_auto_attach(None).await?;
        downloads::configure(&cdp, &spec.download_dir).await?;

        let inner = Arc::new(Inner {
            cdp,
            download_dir: spec.download_dir.clone(),
            console_buffer: spec.console_buffer,
            network_buffer: spec.network_buffer,
            profile_view: EmulateOptions {
                viewport: spec.settings.viewport.clone(),
                geolocation: spec.settings.geolocation.clone(),
                ..EmulateOptions::default()
            },
            content_defaults: Mutex::default(),
            tabs: Mutex::default(),
            routes: Mutex::default(),
        });
        let (ready_tx, ready) = oneshot::channel();
        tokio::spawn(targets::run(
            inner.clone(),
            cdp_events,
            events_tx,
            Mode::Launch(ready_tx),
        ));

        for path in &spec.extensions {
            let path = path.to_str().ok_or_else(|| Error::InvalidArgument {
                field: EXTENSIONS_FIELD.to_owned(),
                message: "extension path is not valid UTF-8".to_owned(),
            })?;
            inner
                .cdp
                .call(None, "Extensions.loadUnpacked", json!({ "path": path }))
                .await?;
        }

        let first_target_id = match tokio::time::timeout(spec.start_timeout, ready).await {
            Err(_) => {
                return Err(Error::Timeout {
                    after_ms: millis(spec.start_timeout),
                });
            }
            Ok(Err(_)) => return Err(internal("browser closed before its first tab attached")),
            Ok(Ok(target_id)) => target_id,
        };

        Ok(Launched {
            driver: Driver::Chromium(Chromium { inner }),
            events,
            pid,
            browser_version,
            first_target_id,
            xvfb_pid,
        })
    }

    fn tab(&self, target_id: &TargetId) -> Result<(SessionId, Arc<PageState>), Error> {
        self.inner
            .tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(target_id)
            .map(|tab| (tab.session_id.clone(), tab.state.clone()))
            .ok_or_else(|| Error::NotFound {
                what: "tab target".to_owned(),
            })
    }

    pub(crate) fn page(&self, target_id: &TargetId) -> Result<Page, Error> {
        let (session_id, state) = self.tab(target_id)?;

        Ok(Page::new(
            Session::Chromium(PageSession {
                cdp: self.inner.cdp.clone(),
                session_id,
                target_id: target_id.clone(),
            }),
            state,
        ))
    }

    // Stores the defaults for Tabs that open later and returns the pages that exist now.
    pub(crate) fn adopt_content_defaults(&self, options: &ContentOptions) -> Vec<Page> {
        *self
            .inner
            .content_defaults
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(options.clone());

        let target_ids: Vec<TargetId> = self
            .inner
            .tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();

        target_ids
            .iter()
            .filter_map(|target_id| self.page(target_id).ok())
            .collect()
    }

    pub(crate) async fn open_tab(&self, url: &str) -> Result<TargetId, Error> {
        self.inner.cdp.create_target(url, true).await
    }

    pub(crate) async fn close_tab(&self, target_id: &TargetId) -> Result<(), Error> {
        self.inner.cdp.close_target(target_id).await
    }

    pub(crate) async fn activate_tab(&self, target_id: &TargetId) -> Result<(), Error> {
        self.inner.cdp.activate_target(target_id).await
    }

    pub(crate) async fn navigate(
        &self,
        target_id: &TargetId,
        url: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<(), Error> {
        let (session_id, _) = self.tab(target_id)?;
        let wanted = wanted_lifecycle(wait_until);

        let navigation = async {
            // Register before Page.navigate. Until the reply names the new loader id, events
            // are only recorded, so one from the previous document cannot resolve the waiter.
            let pending = Arc::new(Mutex::new(Pending::default()));
            let waiter = lifecycle_waiter(&self.inner.cdp, &session_id, wanted, &pending);

            let result = self.inner.cdp.navigate(&session_id, url).await?;
            if let Some(net_error) = result.error_text {
                return Err(Error::NavigationFailed { net_error });
            }
            // Same-document navigation fires no new lifecycle event.
            let Some(loader_id) = result.loader_id else {
                return Ok(());
            };

            {
                let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
                if pending.seen.contains(&loader_id) {
                    return Ok(());
                }
                pending.expected = Some(loader_id);
            }

            waiter
                .await
                .map(|_| ())
                .map_err(|_| internal(CONNECTION_CLOSED))
        };

        tokio::time::timeout(timeout, navigation)
            .await
            .map_err(|_| Error::Timeout {
                after_ms: millis(timeout),
            })?
    }

    pub(crate) async fn cookies(&self) -> Result<Vec<Cookie>, Error> {
        let mut result = self
            .inner
            .cdp
            .call(None, "Storage.getCookies", json!({}))
            .await?;

        parse_cookies(&mut result)
    }

    pub(crate) async fn set_cookies(&self, cookies: &[Cookie]) -> Result<u32, Error> {
        self.inner
            .cdp
            .call(
                None,
                "Storage.setCookies",
                json!({ "cookies": cookie_params(cookies) }),
            )
            .await?;

        Ok(u32::try_from(cookies.len()).unwrap_or(u32::MAX))
    }

    // Chromium may answer or just close the pipe; the Exited event is the real outcome.
    pub(crate) async fn close(&self) -> Result<(), Error> {
        let _ = self.inner.cdp.close_browser().await;

        Ok(())
    }
}

#[derive(Default)]
struct Pending {
    expected: Option<String>,
    seen: Vec<String>,
}

fn lifecycle_waiter(
    cdp: &Cdp,
    session_id: &SessionId,
    wanted: &'static str,
    pending: &Arc<Mutex<Pending>>,
) -> oneshot::Receiver<Value> {
    let pending = pending.clone();

    cdp.wait_for(
        Some(session_id),
        "Page.lifecycleEvent",
        Box::new(move |params| {
            if params["name"] != wanted {
                return false;
            }
            let Some(loader_id) = params["loaderId"].as_str() else {
                return false;
            };
            let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);

            match &pending.expected {
                Some(expected) => expected == loader_id,
                None => {
                    pending.seen.push(loader_id.to_owned());
                    false
                }
            }
        }),
    )
}
