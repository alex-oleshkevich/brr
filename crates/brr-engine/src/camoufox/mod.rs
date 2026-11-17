mod capture;
mod contexts;
mod downloads;
mod emulation;
mod input;
mod intercept;
mod network;
pub mod profile_files;
mod screencast;
mod session;
mod state;
mod targets;

use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use brr_camoufox::fingerprints::{IdentityInput, TargetOs};
use brr_camoufox::launch::{LaunchInput, Position, plan};
use brr_core::{Display, Error, FingerprintOs, Size, TargetId, WaitUntil, WebRtcPolicy};
use brr_juggler::Juggler;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde_json::{Map, Value, json};
use tokio::process::Command;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::time::Instant;

use crate::common::{
    BLANK_PAGE, ENV_DAEMON, ENV_DISPLAY, ENV_INSTANCE_ID, ENV_TZ, ENV_WAYLAND_DISPLAY,
    ENV_XAUTHORITY, EVENTS_QUEUE, die_with_parent, gone, internal, millis,
};
use crate::page::{ContentOptions, Page, PageState, Session};
use crate::xvfb::{DEFAULT_VIRTUAL_SCREEN, Xvfb};
use crate::{
    BROWSER_CLOSED, CamoufoxSpec, Driver, DriverEvent, LaunchSpec, Launched, Navigated, sandbox,
};

pub(crate) use session::{PageSession, init_session, utility_context};

// Firefox's own guard against opening the Safe Mode dialog after crashes.
const ENV_DISABLE_SAFE_MODE: &str = "MOZ_DISABLE_SAFE_MODE_KEY";
// Firefox misreads these; Playwright removes them too.
const ENV_SNAP: [&str; 2] = ["SNAP_NAME", "SNAP_INSTANCE_NAME"];
const APPLICATION_INI: &str = "application.ini";
const VERSION_FIELD: &str = "Version=";
// Camoufox opens its first page on its own; one that has not attached by now is not coming.
const FIRST_PAGE_WAIT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);
const TAB_WAIT: Duration = Duration::from_secs(10);
// network.proxy.type: proxies configured by hand.
const PROXY_TYPE_MANUAL: i64 = 1;

#[derive(Clone)]
pub struct Camoufox {
    inner: Arc<Inner>,
}

struct Inner {
    juggler: Juggler,
    pid: u32,
    download_dir: PathBuf,
    console_buffer: usize,
    network_buffer: usize,
    // Applied to every page that attaches.
    profile_viewport: Option<Size>,
    // Content-Length of attachment responses by URL, until their download is announced.
    download_lengths: Mutex<HashMap<String, u64>>,
    active: Mutex<downloads::Active>,
    // What a Tab without options of its own does to its pages; copied into each new Tab.
    content_defaults: Mutex<Option<ContentOptions>>,
    tabs: Mutex<HashMap<TargetId, Tab>>,
    // Woken when a Tab is registered.
    registered: Notify,
    exited: watch::Receiver<bool>,
}

struct Tab {
    session: PageSession,
    state: Arc<PageState>,
}

pub(crate) fn unsupported() -> Error {
    internal("this operation is not available on Camoufox")
}

fn target_os(os: FingerprintOs) -> TargetOs {
    match os {
        FingerprintOs::Windows => TargetOs::Windows,
        FingerprintOs::Macos => TargetOs::Macos,
        FingerprintOs::Linux => TargetOs::Linux,
    }
}

// The Firefox major version of the Build: `Version=158.0.1-beta.41` in application.ini.
fn firefox_major(install_dir: &Path) -> Result<u32, Error> {
    let ini = std::fs::read_to_string(install_dir.join(APPLICATION_INI)).map_err(|error| {
        Error::BuildMissing {
            reason: format!("{APPLICATION_INI} is not readable: {error}"),
        }
    })?;

    ini.lines()
        .find_map(|line| line.strip_prefix(VERSION_FIELD))
        .and_then(|version| version.split('.').next())
        .and_then(|major| major.parse().ok())
        .ok_or_else(|| Error::BuildMissing {
            reason: format!("{APPLICATION_INI} names no Firefox version"),
        })
}

fn command(spec: &LaunchSpec, camoufox: &CamoufoxSpec) -> Result<Planned, Error> {
    let install_dir = spec
        .executable
        .parent()
        .ok_or_else(|| internal("the Camoufox executable has no directory"))?;
    let settings = &spec.settings;
    let fingerprint = settings.fingerprint.clone().unwrap_or_default();
    std::fs::create_dir_all(&spec.user_data_dir).map_err(internal)?;
    if let Some(dir) = spec.log_file.parent() {
        std::fs::create_dir_all(dir).map_err(internal)?;
    }

    let user_prefs = match &spec.sandbox {
        // The relay is the container's only way out: a Profile's own proxy prefs must not route
        // around it, and loopback goes through it as on Chromium.
        Some(_) => {
            let mut prefs = settings.prefs.clone();
            prefs.extend(
                [
                    ("network.proxy.type", json!(PROXY_TYPE_MANUAL)),
                    ("network.proxy.http", json!(sandbox::RELAY_HOST)),
                    ("network.proxy.http_port", json!(sandbox::RELAY_PORT)),
                    ("network.proxy.ssl", json!(sandbox::RELAY_HOST)),
                    ("network.proxy.ssl_port", json!(sandbox::RELAY_PORT)),
                    ("network.proxy.no_proxies_on", json!("")),
                    ("network.proxy.allow_hijacking_localhost", json!(true)),
                ]
                .map(|(name, value)| (name.to_owned(), value)),
            );

            Cow::Owned(prefs)
        }
        None => Cow::Borrowed(&settings.prefs),
    };

    let input = LaunchInput {
        install_dir,
        fonts_dir: &camoufox.fonts_dir,
        addons: &camoufox.addons,
        model: &camoufox.model,
        identity: IdentityInput {
            os: fingerprint.os.map(target_os),
            screen: settings.screen.clone().or(fingerprint.screen),
            fonts: fingerprint.fonts,
            webgl: fingerprint
                .webgl
                .map(|webgl| (webgl.vendor, webgl.renderer)),
            locale: settings.locale.clone(),
            ff_version: firefox_major(install_dir)?,
        },
        geo: camoufox.geo.clone(),
        proxied: spec.proxy_server.is_some(),
        timezone: settings.timezone.clone(),
        geolocation: settings.geolocation.as_ref().map(|position| Position {
            latitude: position.latitude,
            longitude: position.longitude,
            accuracy: position.accuracy,
        }),
        // Any WebRTC policy but the default keeps the browser from advertising addresses.
        block_webrtc: settings.webrtc != WebRtcPolicy::Default,
        headless: spec.display == Display::Headless,
        // Page::evaluate runs in the page's own world, as on Chromium.
        main_world_eval: true,
        user_prefs: &user_prefs,
        seed: camoufox.seed,
        work_dir: &spec.user_data_dir,
    };
    let plan = plan(&input)?;

    let mut args = plan.args.clone();
    if spec.display != Display::Headless {
        args.push("-wait-for-browser".into());
        args.push("-foreground".into());
    }
    args.push("-profile".into());
    args.push(spec.user_data_dir.clone().into_os_string());
    if let Some(Size { width, height }) = &plan.window {
        args.extend(
            ["-width", &width.to_string(), "-height", &height.to_string()].map(OsString::from),
        );
    }
    args.extend(settings.args.iter().map(OsString::from));
    if spec.display == Display::Headless {
        args.push(BLANK_PAGE.into());
    }

    // A file, never a pipe: an unread full pipe blocks the browser.
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&spec.log_file)
        .map_err(internal)?;
    let log_err = log.try_clone().map_err(internal)?;

    let mut command = Command::new(&spec.executable);
    command.args(args);
    if let Some(timezone) = &settings.timezone {
        command.env(ENV_TZ, timezone);
    }
    // The config chunks total hundreds of KB: they go through the environment, never a shell.
    command
        .envs(plan.env.clone())
        .envs(&settings.env)
        .env(ENV_DISABLE_SAFE_MODE, "1")
        .env(ENV_INSTANCE_ID, spec.instance_id.to_string())
        .env(ENV_DAEMON, "1")
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err)
        .process_group(0)
        .kill_on_drop(false);
    die_with_parent(&mut command);
    for name in ENV_SNAP {
        command.env_remove(name);
    }
    if spec.display == Display::Visible {
        if let Some(display) = &spec.display_env.display {
            command.env(ENV_DISPLAY, display);
        }
        if let Some(display) = &spec.display_env.wayland_display {
            command.env(ENV_WAYLAND_DISPLAY, display);
        }
    }

    // The browser's window is sized from this screen, so a virtual display must not be smaller.
    let dimension = |key: &str| u32::try_from(plan.config.get(key)?.as_u64()?).ok();
    let screen = dimension("screen.width")
        .zip(dimension("screen.height"))
        .map(|(width, height)| Size { width, height });

    Ok(Planned {
        command,
        user_prefs: plan.user_prefs,
        screen,
    })
}

struct Planned {
    command: Command,
    user_prefs: Map<String, Value>,
    // The screen the browser will report: the identity's, which the Profile's may have chosen.
    screen: Option<Size>,
}

impl Camoufox {
    pub(crate) async fn launch(spec: LaunchSpec) -> Result<Launched, Error> {
        let camoufox = spec
            .camoufox
            .clone()
            .ok_or_else(|| internal("a Camoufox launch needs its assets"))?;
        let Planned {
            mut command,
            user_prefs,
            screen,
        } = command(&spec, &camoufox)?;
        // The display is made after the plan: its screen is the one the browser will use.
        let screen =
            (spec.display == Display::Virtual).then(|| screen.unwrap_or(DEFAULT_VIRTUAL_SCREEN));
        // In the docker sandbox the runner starts Xvfb inside the container.
        let xvfb = match (&screen, &spec.sandbox) {
            (Some(screen), None) => {
                let xvfb = Xvfb::start(screen.clone(), &spec.instance_id, &spec.log_file).await?;
                command
                    .env(ENV_DISPLAY, xvfb.display())
                    .env(ENV_XAUTHORITY, xvfb.auth_file())
                    .env_remove(ENV_WAYLAND_DISPLAY);

                Some(xvfb)
            }
            _ => None,
        };
        let xvfb_pid = xvfb.as_ref().map(Xvfb::pid);

        let spawned = match &spec.sandbox {
            Some(sandbox) => sandbox::start(&spec, sandbox, &command, screen).await,
            None => brr_proto::spawn(command)
                .map(|spawned| (spawned, None))
                .map_err(internal),
        };
        let (spawned, remote) = match spawned {
            Ok(spawned) => spawned,
            Err(error) => {
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
        let (exited_tx, exited) = watch::channel(false);

        let exit_tx = events_tx.clone();
        tokio::spawn(async move {
            let (code, signal) = match child.wait().await {
                Ok(status) => (status.code(), status.signal()),
                Err(_) => (None, None),
            };
            // Before the display is stopped: close() reads this to know the pid is gone.
            let _ = exited_tx.send(true);
            if let Some(mut xvfb) = xvfb {
                xvfb.stop().await;
            }
            let _ = exit_tx.send(DriverEvent::Exited { code, signal }).await;
        });

        let (juggler, juggler_events) = Juggler::start(spawned.mux, spawned.events);
        let inner = Arc::new(Inner {
            juggler: juggler.clone(),
            pid,
            download_dir: spec.download_dir.clone(),
            console_buffer: spec.console_buffer,
            network_buffer: spec.network_buffer,
            profile_viewport: spec.settings.viewport.clone(),
            download_lengths: Mutex::default(),
            active: Mutex::default(),
            content_defaults: Mutex::default(),
            tabs: Mutex::default(),
            registered: Notify::new(),
            exited,
        });
        let (ready_tx, mut ready) = oneshot::channel();
        tokio::spawn(targets::run(
            inner.clone(),
            juggler_events,
            events_tx,
            ready_tx,
        ));

        juggler.enable(&user_prefs).await?;
        let browser_version = juggler.browser_version().await?;
        downloads::configure(&juggler, None, &spec.download_dir).await?;

        let first_target_id = Self::first_target(&juggler, &mut ready, spec.start_timeout).await?;

        let camoufox = Camoufox { inner };
        if spec.display != Display::Headless {
            // A headed window's first tab has no document yet (readyState "uninitialized") and
            // never fires a load event; loading the blank page gives it both.
            camoufox
                .navigate(&first_target_id, BLANK_PAGE, WaitUntil::Load, TAB_WAIT)
                .await?;
        }

        Ok(Launched {
            driver: Driver::Camoufox(camoufox),
            events: match remote {
                Some(remote) => remote.relay(events),
                None => events,
            },
            pid,
            browser_version,
            first_target_id,
            xvfb_pid,
            tunnels: spawned.opened,
        })
    }

    async fn first_target(
        juggler: &Juggler,
        ready: &mut oneshot::Receiver<TargetId>,
        start_timeout: Duration,
    ) -> Result<TargetId, Error> {
        let closed = || internal(BROWSER_CLOSED);
        let timeout = |_| Error::Timeout {
            after_ms: millis(start_timeout),
        };

        let first = FIRST_PAGE_WAIT.min(start_timeout);
        if let Ok(attached) = tokio::time::timeout(first, &mut *ready).await {
            return attached.map_err(|_| closed());
        }

        juggler.new_page(None).await?;
        tokio::time::timeout(start_timeout.saturating_sub(first), ready)
            .await
            .map_err(timeout)?
            .map_err(|_| closed())
    }

    fn tab(&self, target_id: &TargetId) -> Result<(PageSession, Arc<PageState>), Error> {
        self.inner
            .tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(target_id)
            .map(|tab| (tab.session.clone(), tab.state.clone()))
            .ok_or_else(|| gone("tab target"))
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

    pub(crate) fn page(&self, target_id: &TargetId) -> Result<Page, Error> {
        let (session, state) = self.tab(target_id)?;

        Ok(Page::new(Session::Camoufox(session), state))
    }

    // A page that has attached knows its main frame; before that nothing can be driven.
    async fn attached_tab(
        &self,
        target_id: &TargetId,
    ) -> Result<(PageSession, Arc<PageState>), Error> {
        // Each wait is created before the check it follows, so a change in between is not missed.
        let found = tokio::time::timeout(TAB_WAIT, async {
            loop {
                let registered = self.inner.registered.notified();
                let Ok((session, state)) = self.tab(target_id) else {
                    registered.await;
                    continue;
                };
                let changed = state.changed.notified();
                if state.main_frame().is_empty() {
                    changed.await;
                    continue;
                }

                drop(changed);

                return (session, state);
            }
        })
        .await;

        found.map_err(|_| Error::Timeout {
            after_ms: millis(TAB_WAIT),
        })
    }

    pub(crate) async fn open_tab(&self, url: &str) -> Result<TargetId, Error> {
        self.open_tab_with(None, url).await
    }

    async fn open_tab_with(&self, context_id: Option<&str>, url: &str) -> Result<TargetId, Error> {
        let target_id = self.inner.juggler.new_page(context_id).await?;
        if url != BLANK_PAGE {
            // The Tab exists once it is created; the page loads on its own, as on Chromium.
            let (session, state) = self.attached_tab(&target_id).await?;
            self.inner
                .juggler
                .navigate(&session.juggler_session, &state.main_frame(), url)
                .await?;
        }

        Ok(target_id)
    }

    pub(crate) async fn close_tab(&self, target_id: &TargetId) -> Result<(), Error> {
        let (session, _) = self.tab(target_id)?;

        self.inner
            .juggler
            .close_page(&session.juggler_session)
            .await
    }

    pub(crate) async fn activate_tab(&self, target_id: &TargetId) -> Result<(), Error> {
        let (session, _) = self.tab(target_id)?;

        session
            .call("Page.bringToFront", json!({}))
            .await
            .map(|_| ())
    }

    pub(crate) async fn navigate(
        &self,
        target_id: &TargetId,
        url: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<Navigated, Error> {
        let (session, state) = self.attached_tab(target_id).await?;
        let started = Instant::now();

        let navigated = session
            .navigate(&state.main_frame(), url, wait_until, timeout)
            .await?;
        // A download's own transfer would hold the idle wait for as long as it runs.
        if wait_until == WaitUntil::Networkidle && navigated == Navigated::Page {
            state.wait_network_idle(started, timeout).await?;
        }

        Ok(navigated)
    }

    // Juggler answers Browser.close with an EOF, not a reply, and a hung browser answers nothing:
    // the request is sent without waiting, and a browser still alive after the wait is killed.
    pub(crate) async fn close(&self) -> Result<(), Error> {
        let juggler = self.inner.juggler.clone();
        tokio::spawn(async move {
            let _ = juggler.close_browser().await;
        });

        let mut exited = self.inner.exited.clone();
        if tokio::time::timeout(CLOSE_TIMEOUT, exited.wait_for(|exited| *exited))
            .await
            .is_err()
            && let Ok(pid) = i32::try_from(self.inner.pid)
        {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
