pub(crate) mod idle;
mod owner;
pub(crate) mod promote;

use std::{
    cmp::Ordering,
    collections::HashMap,
    fs::{self, DirBuilder, File},
    io::{ErrorKind, Read, Seek, SeekFrom},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
    time::Duration,
};

use brr_cdp::{CONNECTION_CLOSED, Downstream, Outbound};
use brr_core::{
    AdmissionMode, Build, BuildId, BuildSource, ConflictReason, Cookie, Display, EndReason, Engine,
    Error, EventData, Host, HumanReason, IdleOverrides, Instance, InstanceExtension, InstanceId,
    InstanceKind, InstanceState, InstanceTrigger, Profile, ProfileId, ProfileKind, ProfileSettings,
    ProxyId, SandboxMode, Tab, TabCloseReason, TabId, TabState, WaitUntil, compare_versions,
    now_ms,
};
use brr_engine::{
    AttachSpec, BROWSER_CLOSED, CamoufoxSpec, Connection, ContextId, Driver, LaunchSpec, Navigated,
    devtools_active_port, page::Page, sandbox::SandboxSpec,
};
use brr_proto::frames::Stream;
use brr_store::{build, instance};
use futures::future::join_all;
use tokio::io::DuplexStream;
use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    time,
};
use tracing::warn;

use crate::{
    app::App,
    internal,
    services::{
        STATE_CHANGED,
        admission::{AdmitRequest, Cancel, cancelled},
        camoufox,
        events::{self, Published},
        extensions, leases,
        profiles::resolve_build,
        proxies, quota, relay, sandbox,
    },
};
use owner::{Activity, Command, Inbox, Reply, fail_admission, fail_launch, stop_unowned};

pub(crate) use owner::transition;

#[cfg(test)]
mod tests;

const COMMAND_QUEUE: usize = 64;
const ACTIVITY_QUEUE: usize = 256;
const BLANK_PAGE: &str = "about:blank";
const LAST_VERSION_FILE: &str = "Last Version";
const SINGLETON_FILES: [&str; 3] = ["SingletonLock", "SingletonSocket", "SingletonCookie"];
const SHUTDOWN_MARGIN: Duration = Duration::from_secs(5);
const CANCEL_WAIT: Duration = Duration::from_secs(10);
// Existing pages of a Live browser reach Ready from the driver's own events.
const ATTACH_TABS_WAIT: Duration = Duration::from_secs(10);
const LOG_TAIL_BYTES: u64 = 16 * 1024;
const LOG_LINE_MAX_CHARS: usize = 200;
// Chromium prefixes each log line with `[pid:tid:time:LEVEL:file(line)] `.
const CHROMIUM_PREFIX_END: &str = "] ";
const CHROMIUM_FATAL: &str = ":FATAL:";
const CHROMIUM_ERROR: &str = ":ERROR:";
// Lines Chromium prints after the real cause; they say nothing about what went wrong.
const GENERIC_EXITS: [&str; 2] = ["failed to initialize", "Exiting"];
const VISIBLE_EXIT_HINT: &str =
    "use display headless or virtual, or check that the desktop display is reachable";
// Chromium's own sandbox needs unprivileged user namespaces, which hardened hosts block.
const NO_USABLE_SANDBOX: &str = "No usable sandbox";
const CHROMIUM_SANDBOX: &str = "Chromium sandbox";
const NO_USABLE_SANDBOX_HELP: &str = "this host blocks the user namespaces Chromium's own sandbox needs; set Settings → Sandbox to Docker, which isolates each browser in its own container";
// Browser.getVersion's product of a headless Chrome, e.g. `HeadlessChrome/131.0.6778.85`.
const HEADLESS_PRODUCT: &str = "HeadlessChrome";
// Holds the forwarder's socket, which the container sees too.
const RUN_DIR_MODE: u32 = 0o700;
const VISIBLE_IN_SANDBOX: &str =
    "visible display is not available in the docker sandbox; use headless or virtual";
// How the engine's failed `docker` calls begin: on a Host, SSH or the Host's Docker failed.
const DOCKER_FAILED_PREFIX: &str = "`docker ";

pub struct LaunchRequest {
    pub profile: Profile,
    pub display: Option<Display>,
    pub priority: u8,
    pub admission: AdmissionMode,
    pub force: bool,
}

pub enum AttachSource {
    CdpUrl(String),
    InstallName(String),
    Extension(DuplexStream),
}

#[derive(Debug, Clone)]
pub struct InstanceView {
    pub instance: Instance,
    pub tabs: Vec<Tab>,
    pub pinned: bool,
}

struct Handle {
    commands: mpsc::Sender<Command>,
    activity: mpsc::Sender<Activity>,
    view: watch::Receiver<InstanceView>,
    driver: watch::Receiver<Option<Driver>>,
    // The extension directories its browser loaded; they must outlive it.
    extensions: Vec<PathBuf>,
}

#[derive(Default)]
pub struct Instances {
    owners: Mutex<HashMap<InstanceId, Handle>>,
    // Launches that have no owner yet: a stop reaches them through this signal.
    launching: Mutex<HashMap<InstanceId, watch::Sender<Option<EndReason>>>>,
    // Set by shutdown: no launch or attach starts after it.
    closed: AtomicBool,
}

struct Launching<'a> {
    instances: &'a Instances,
    instance_id: InstanceId,
}

impl Drop for Launching<'_> {
    fn drop(&mut self) {
        self.instances
            .launching
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.instance_id);
    }
}

// What the launch needs before it asks for a slot.
struct Prepared {
    proxy_server: Option<String>,
    proxy_id: Option<ProxyId>,
    extensions: Vec<PathBuf>,
    loaded: Vec<InstanceExtension>,
    settings: ProfileSettings,
    camoufox: Option<CamoufoxSpec>,
    sandbox: Option<SandboxSpec>,
}

impl Instances {
    // The launch outlives its caller: a dropped request must not leave a row queued forever.
    pub async fn launch(
        &self,
        app: &Arc<App>,
        req: LaunchRequest,
    ) -> Result<(Instance, Tab), Error> {
        let app = Arc::clone(app);

        tokio::spawn(events::keep_actor(async move {
            app.instances.run_launch(&app, req).await
        }))
        .await
        .map_err(internal)?
    }

    async fn run_launch(
        &self,
        app: &Arc<App>,
        req: LaunchRequest,
    ) -> Result<(Instance, Tab), Error> {
        // Registered before the gate is read: shutdown either sees this launch or it sees the gate.
        let instance_id = InstanceId::new();
        let (_launching, mut cancel) = self.begin_launch(instance_id);
        self.ensure_open()?;

        let profile = req.profile;
        quota::check_launch(app, &profile).await?;
        let display = req.display.unwrap_or(profile.settings.display);
        let sandbox_mode = app.settings().sandbox.mode;
        if sandbox_mode == SandboxMode::Docker && display == Display::Visible {
            return Err(Error::InvalidArgument {
                field: "display".to_owned(),
                message: VISIBLE_IN_SANDBOX.to_owned(),
            });
        }
        let choice = profile.settings.build.clone();
        let build = app.store.run(move |c| resolve_build(c, &choice)).await?;
        let persistent = profile.kind == ProfileKind::Persistent;

        let data_dir = if persistent {
            let data_dir = profile
                .data_dir
                .clone()
                .ok_or_else(|| internal(format!("profile {} has no data dir", profile.id)))?;
            if !req.force {
                ensure_no_downgrade(&data_dir, &build)?;
            }

            data_dir
        } else {
            app.paths.ephemeral_dir().join(instance_id.to_string())
        };

        let now = now_ms();
        let mut instance = Instance {
            id: instance_id,
            kind: InstanceKind::Managed,
            profile_id: Some(profile.id),
            build_id: Some(build.id),
            state: InstanceState::Queued,
            display,
            priority: req.priority,
            admission: req.admission,
            pid: None,
            log_path: Some(app.paths.instance_log(&instance_id)),
            cdp_url: None,
            data_dir: Some(data_dir.clone()),
            ephemeral: !persistent,
            promote_to_name: None,
            restart_count: 0,
            last_activity_at: now,
            queued_at: now,
            started_at: None,
            ended_at: None,
            end_reason: None,
            error: None,
            settings: Some(profile.settings.clone()),
            template_id: profile.template_id,
            template_name: profile.template_name.clone(),
            extensions: None,
            proxy_id: None,
            sandbox: Some(sandbox_mode),
            traffic: None,
            host_id: None,
            profile_name: Some(profile.name.clone()),
            build: None,
            tab_count: 0,
            lease_count: 0,
            rss_bytes: None,
            capabilities: Vec::new(),
        };

        // The lock check and the insert share one transaction, so two launches cannot both pass.
        let row = instance.clone();
        let profile_id = profile.id;
        // Checked inside the lock check's transaction, after the exporter took its guard.
        let exporting = app.exports.is_exporting(&profile_id);
        app.store
            .run(move |c| {
                let tx = c.transaction().map_err(internal)?;
                if persistent && exporting {
                    return Err(Error::Conflict {
                        reason: ConflictReason::ProfileLocked,
                        detail: Some(
                            "a backup or export is running; retry when it finishes".to_string(),
                        ),
                    });
                }
                if persistent && instance::active_for_profile(&tx, &profile_id)?.is_some() {
                    return Err(Error::Conflict {
                        reason: ConflictReason::ProfileLocked,
                        detail: None,
                    });
                }
                instance::insert(&tx, &row)?;
                tx.commit().map_err(internal)
            })
            .await?;
        app.events
            .publish(EventData::InstanceStateChanged {
                instance_id,
                profile_id: instance.profile_id,
                from: None,
                to: InstanceState::Queued,
                trigger: InstanceTrigger::Launch,
                reason: None,
            })
            .await?;

        let prepared = tokio::select! {
            prepared = prepare(app, &instance_id, &profile, &build, sandbox_mode) => prepared,
            _ = cancelled(&mut cancel) => return end_unadmitted(app, &mut instance, req.admission).await,
        };
        let Prepared {
            proxy_server,
            proxy_id,
            extensions,
            loaded,
            settings,
            camoufox: camoufox_spec,
            sandbox: mut sandbox_spec,
        } = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                fail_launch(app, &mut instance, &error).await;

                return Err(error);
            }
        };
        // Written with the next state change, which every path from here makes.
        instance.extensions = Some(loaded);
        instance.proxy_id = proxy_id;

        let admitted = app
            .admission
            .admit(
                app,
                AdmitRequest {
                    sandbox: sandbox_mode,
                    instance_id,
                    profile_name: profile.name.clone(),
                    engine: build.engine,
                    display,
                    tags: profile.tags.clone(),
                    priority: req.priority,
                    mode: req.admission,
                    queued_at: now,
                },
                &mut cancel,
            )
            .await;
        let placed = match admitted {
            Ok(placed) => placed,
            Err(error) => {
                proxies::release(app, &instance_id);
                sandbox::remove_run_dir(&app.paths, &instance_id);
                fail_admission(app, &mut instance, req.admission, &error).await;

                return Err(error);
            }
        };
        // Only a docker launch is placed on a Host, so a placed run has a docker target.
        if let (Some(host), Some(sandbox)) = (&placed, sandbox_spec.as_mut()) {
            instance.host_id = Some(host.id);
            sandbox.docker.docker_url = Some(host.docker_url.clone());
        }
        if let Err(error) = transition(app, &mut instance, InstanceTrigger::Admitted, None).await {
            fail_launch(app, &mut instance, &error).await;
            app.admission.finished(app, &instance_id).await;

            return Err(error);
        }
        // Admitted just as the stop came: the slot goes back unused.
        let cancelled = *cancel.borrow();
        if let Some(reason) = cancelled {
            stop_unowned(app, &mut instance, reason).await;
            app.admission.finished(app, &instance_id).await;

            return Err(cancelled_launch());
        }

        let spec = LaunchSpec {
            instance_id,
            engine: build.engine,
            executable: build.path,
            user_data_dir: data_dir.clone(),
            log_file: app.paths.instance_log(&instance_id),
            display,
            display_env: app.display_env.clone(),
            settings,
            start_timeout: Duration::from_secs(app.settings().instance.start_timeout_s),
            extensions,
            proxy_server,
            download_dir: app.paths.downloads_dir().join(instance_id.to_string()),
            console_buffer: app.settings().tab.console_buffer,
            network_buffer: app.settings().tab.network_buffer,
            camoufox: camoufox_spec,
            sandbox: sandbox_spec,
        };

        // Computed before the process exists: the list wait must not sit between spawning the
        // browser and tracking it.
        let options = app
            .content
            .options(app, spec.settings.block_ads, spec.settings.dismiss_cookies)
            .await;
        // A Host gets the image, browser and run volume first; this may take minutes the first
        // time, so it runs before the start timeout and yields to a stop.
        let host_ready = tokio::select! {
            ready = brr_engine::sandbox::prepare(&spec) => ready,
            reason = crate::services::admission::cancelled(&mut cancel) => {
                stop_unowned(app, &mut instance, reason).await;
                app.admission.finished(app, &instance_id).await;

                return Err(cancelled_launch());
            }
        };
        if let Err(error) = host_ready {
            let error = match &placed {
                Some(host) => host_launch_failed(app, host, &error),
                None => error,
            };
            fail_launch(app, &mut instance, &error).await;
            app.admission.finished(app, &instance_id).await;

            return Err(error);
        }
        let started = time::Instant::now();
        let launched = match start_browser(&spec, persistent).await {
            Ok(launched) => launched,
            Err(error) => {
                let error = if closed_while_starting(&error) {
                    exited_while_starting(&spec).unwrap_or(error)
                } else {
                    error
                };
                let error = match &placed {
                    Some(host) => host_launch_failed(app, host, &error),
                    None => error,
                };
                fail_launch(app, &mut instance, &error).await;
                app.admission.finished(app, &instance_id).await;

                return Err(error);
            }
        };
        serve_tunnels(app, instance_id, launched.tunnels);

        // Before the first navigation any Client can make.
        if let Err(error) = launched.driver.set_content_defaults(options).await {
            warn!(%instance_id, %error, "content defaults not applied");
        }

        app.supervisor.track(launched.pid);
        if let Some(xvfb_pid) = launched.xvfb_pid {
            app.supervisor.track_xvfb(instance_id, xvfb_pid);
        }
        instance.pid = Some(launched.pid);
        instance.started_at = Some(now_ms());
        // A stop that came while the browser started: it goes down before any owner exists.
        let cancelled = *cancel.borrow();
        if let Some(reason) = cancelled {
            app.supervisor
                .kill_instance(&instance_id, instance.pid)
                .await;
            stop_unowned(app, &mut instance, reason).await;
            app.admission.finished(app, &instance_id).await;

            return Err(cancelled_launch());
        }
        let (commands, command_rx) = mpsc::channel(COMMAND_QUEUE);
        let (activity, activity_rx) = mpsc::channel(ACTIVITY_QUEUE);
        let (view_tx, view) = watch::channel(InstanceView {
            instance: instance.clone(),
            tabs: Vec::new(),
            pinned: profile.pinned,
        });
        let (driver_tx, driver_rx) = watch::channel(Some(launched.driver.clone()));
        self.owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                instance_id,
                Handle {
                    commands,
                    activity,
                    view,
                    extensions: spec.extensions.clone(),
                    driver: driver_rx,
                },
            );
        let start_timeout = spec.start_timeout;
        app.admission.ready(app, &instance_id).await;
        owner::spawn(
            Arc::clone(app),
            instance.clone(),
            spec,
            launched.events,
            Inbox {
                commands: command_rx,
                activity: activity_rx,
            },
            view_tx,
            driver_tx,
        );

        // A stop that slipped in after the last check finds the owner now.
        let cancelled = *cancel.borrow();
        if let Some(reason) = cancelled {
            if let Err(error) = self.stop(app, &instance_id, reason).await {
                warn!(%instance_id, %error, "cancelled launch not stopped");
            }

            return Err(cancelled_launch());
        }

        let ready = self.call(&instance_id, |reply| Command::Ready {
            target_id: launched.first_target_id,
            reply,
        });
        // One deadline for the browser and its first page.
        let waited = time::timeout(start_timeout.saturating_sub(started.elapsed()), ready).await;
        // The Instance stays starting until its first page is ready, so a timeout ends it failed.
        let failure = match waited {
            Ok(Ok(tab)) => match self
                .call(&instance_id, |reply| Command::Started { reply })
                .await
            {
                Ok(running) => return Ok((running, tab)),
                Err(error) => error,
            },
            // The first page closed before it was ready, or the owner ended: nothing runs.
            Ok(Err(error)) => error,
            Err(_) => Error::Timeout {
                after_ms: u64::try_from(start_timeout.as_millis()).unwrap_or(u64::MAX),
            },
        };

        // An owner that already ended (the browser died) has nothing left to fail.
        if self.commands(&instance_id).is_some() {
            let failed = self
                .call(&instance_id, |reply| Command::StartFailed {
                    error: failure.clone(),
                    reply,
                })
                .await;
            if let Err(error) = failed {
                warn!(%instance_id, %error, "instance without a ready first page not ended");
            }
        }

        Err(failure)
    }

    // Like launch, the attach outlives its caller: a dropped request must not leave an Attaching
    // row behind.
    pub async fn attach(
        &self,
        app: &Arc<App>,
        source: AttachSource,
    ) -> Result<(Instance, Vec<Tab>), Error> {
        let app = Arc::clone(app);

        tokio::spawn(events::keep_actor(async move {
            app.instances.run_attach(&app, source).await
        }))
        .await
        .map_err(internal)?
    }

    async fn run_attach(
        &self,
        app: &Arc<App>,
        source: AttachSource,
    ) -> Result<(Instance, Vec<Tab>), Error> {
        self.ensure_open()?;
        let instance_id = InstanceId::new();
        let now = now_ms();
        let mut instance = Instance {
            id: instance_id,
            kind: InstanceKind::Live,
            profile_id: None,
            build_id: None,
            state: InstanceState::Attaching,
            display: Display::Visible,
            priority: 0,
            admission: app.settings().admission.default,
            pid: None,
            log_path: None,
            cdp_url: match &source {
                AttachSource::CdpUrl(url) => Some(origin(url)),
                _ => None,
            },
            data_dir: None,
            ephemeral: false,
            promote_to_name: None,
            restart_count: 0,
            last_activity_at: now,
            queued_at: now,
            started_at: None,
            ended_at: None,
            end_reason: None,
            error: None,
            settings: None,
            template_id: None,
            template_name: None,
            extensions: None,
            proxy_id: None,
            sandbox: None,
            traffic: None,
            host_id: None,
            profile_name: None,
            build: None,
            tab_count: 0,
            lease_count: 0,
            rss_bytes: None,
            capabilities: Vec::new(),
        };

        let row = instance.clone();
        app.store.run(move |c| instance::insert(c, &row)).await?;
        app.events
            .publish(EventData::InstanceStateChanged {
                instance_id,
                profile_id: None,
                from: None,
                to: InstanceState::Attaching,
                trigger: InstanceTrigger::Attach,
                reason: None,
            })
            .await?;

        let attached = match connect(app, source, instance_id).await {
            Ok(attached) => attached,
            Err(error) => {
                fail_attach(app, &mut instance, &error).await;

                return Err(error);
            }
        };

        instance.build_id = matching_build(app, &attached.browser_version).await;
        if attached.browser_version.contains(HEADLESS_PRODUCT) {
            instance.display = Display::Headless;
        }
        instance.started_at = Some(now_ms());
        transition(app, &mut instance, InstanceTrigger::Connected, None).await?;

        let (commands, command_rx) = mpsc::channel(COMMAND_QUEUE);
        let (activity, activity_rx) = mpsc::channel(ACTIVITY_QUEUE);
        let (view_tx, view) = watch::channel(InstanceView {
            instance: instance.clone(),
            tabs: Vec::new(),
            pinned: false,
        });
        let (driver_tx, driver_rx) = watch::channel(Some(attached.driver.clone()));
        self.owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                instance_id,
                Handle {
                    commands,
                    activity,
                    view,
                    extensions: Vec::new(),
                    driver: driver_rx,
                },
            );
        owner::spawn(
            Arc::clone(app),
            instance.clone(),
            live_spec(app, instance_id),
            attached.events,
            Inbox {
                commands: command_rx,
                activity: activity_rx,
            },
            view_tx,
            driver_tx,
        );

        // Shutdown set its gate before it listed the owners: an attach that registered after that
        // list lets go itself.
        if let Err(error) = self.ensure_open() {
            if let Err(detach_error) = self
                .detach_with(app, &instance_id, EndReason::Shutdown)
                .await
            {
                warn!(%instance_id, %detach_error, "late attach not detached");
            }

            return Err(error);
        }

        // The driver announces the existing pages itself, so the Tabs come from the owner.
        let ready = join_all(attached.targets.into_iter().map(|target| {
            self.call(&instance_id, |reply| Command::Ready {
                target_id: target.target_id,
                reply,
            })
        }));
        match time::timeout(ATTACH_TABS_WAIT, ready).await {
            Ok(tabs) => Ok((instance, tabs.into_iter().filter_map(Result::ok).collect())),
            Err(_) => {
                if let Err(error) = self.detach_with(app, &instance_id, EndReason::Stop).await {
                    warn!(%instance_id, %error, "attach without ready tabs not detached");
                }

                Err(Error::Timeout {
                    after_ms: u64::try_from(ATTACH_TABS_WAIT.as_millis()).unwrap_or(u64::MAX),
                })
            }
        }
    }

    // Ends brr's connection; the browser and its pages stay exactly as they are.
    pub async fn detach(
        &self,
        app: &Arc<App>,
        instance_id: &InstanceId,
    ) -> Result<Instance, Error> {
        self.detach_with(app, instance_id, EndReason::Stop).await
    }

    async fn detach_with(
        &self,
        app: &Arc<App>,
        instance_id: &InstanceId,
        reason: EndReason,
    ) -> Result<Instance, Error> {
        let id = *instance_id;
        let row = app.store.run(move |c| instance::get(c, &id)).await?;
        if row.kind != InstanceKind::Live {
            return Err(Error::InvalidArgument {
                field: "instance_id".to_owned(),
                message: "only Live browser Instances can be detached; use instance.stop"
                    .to_owned(),
            });
        }
        if self.commands(instance_id).is_none() {
            row.state.transition(InstanceTrigger::Detach)?;
        }

        self.call(instance_id, |reply| Command::Detach { reason, reply })
            .await
    }

    pub async fn stop(
        &self,
        app: &Arc<App>,
        instance_id: &InstanceId,
        reason: EndReason,
    ) -> Result<Instance, Error> {
        if self.commands(instance_id).is_none()
            && let Some(ended) = self.cancel_launch(app, instance_id, reason)
            // The owner may have appeared since: then it takes the stop below.
            && self.commands(instance_id).is_none()
        {
            return wait_final(app, instance_id, ended).await;
        }

        if let Some(commands) = self.commands(instance_id) {
            let (reply, done) = oneshot::channel();
            if commands.send(Command::Stop { reason, reply }).await.is_ok()
                && let Ok(stopped) = done.await
            {
                return stopped;
            }
        }

        // No owner (or it ended meanwhile): the row says whether the Instance is final.
        let id = *instance_id;
        let row = app.store.run(move |c| instance::get(c, &id)).await?;
        row.state.transition(InstanceTrigger::Stop)?;

        Err(internal(format!(
            "instance {instance_id} is not ready to stop yet; retry"
        )))
    }

    pub async fn promote(&self, instance_id: &InstanceId, name: String) -> Result<Instance, Error> {
        let commands = self.commands(instance_id).ok_or_else(|| Error::NotFound {
            what: format!("running instance {instance_id}"),
        })?;
        let ended = || internal(format!("instance {instance_id} ended before it answered"));

        let (reply, done) = oneshot::channel();
        commands
            .send(Command::Promote { name, reply })
            .await
            .map_err(|_| ended())?;

        done.await.map_err(|_| ended())?
    }

    pub async fn open_tab(
        &self,
        instance_id: &InstanceId,
        url: Option<String>,
    ) -> Result<Tab, Error> {
        let url = url.unwrap_or_else(|| BLANK_PAGE.to_owned());

        self.call(instance_id, |reply| Command::OpenTab { url, reply })
            .await
    }

    pub async fn create_context(&self, instance_id: &InstanceId) -> Result<ContextId, Error> {
        self.call(instance_id, |reply| Command::CreateContext { reply })
            .await
    }

    pub async fn open_tab_in(
        &self,
        instance_id: &InstanceId,
        context_id: &ContextId,
        url: Option<String>,
    ) -> Result<Tab, Error> {
        let url = url.unwrap_or_else(|| BLANK_PAGE.to_owned());

        self.call(instance_id, |reply| Command::OpenTabIn {
            context_id: context_id.clone(),
            url,
            reply,
        })
        .await
    }

    pub async fn dispose_context(
        &self,
        instance_id: &InstanceId,
        context_id: &ContextId,
    ) -> Result<(), Error> {
        self.call(instance_id, |reply| Command::DisposeContext {
            context_id: context_id.clone(),
            reply,
        })
        .await
    }

    pub async fn close_tab(
        &self,
        instance_id: &InstanceId,
        tab_id: &TabId,
        reason: TabCloseReason,
    ) -> Result<Tab, Error> {
        self.call(instance_id, |reply| Command::CloseTab {
            tab_id: *tab_id,
            reason,
            reply,
        })
        .await
    }

    pub async fn close_tab_unless_last(
        &self,
        instance_id: &InstanceId,
        tab_id: &TabId,
        reason: TabCloseReason,
    ) -> Result<Tab, Error> {
        self.call(instance_id, |reply| Command::CloseTabUnlessLast {
            tab_id: *tab_id,
            reason,
            reply,
        })
        .await
    }

    pub async fn set_tab_pinned(
        &self,
        instance_id: &InstanceId,
        tab_id: &TabId,
        pinned: bool,
    ) -> Result<Tab, Error> {
        self.call(instance_id, |reply| Command::SetTabPinned {
            tab_id: *tab_id,
            pinned,
            reply,
        })
        .await
    }

    // Returns when the person is done; a wait that runs out is a Timeout.
    pub async fn await_human(
        &self,
        instance_id: &InstanceId,
        tab_id: &TabId,
        reason: HumanReason,
        timeout: Duration,
    ) -> Result<Tab, Error> {
        leases::expect_take_over();

        self.call(instance_id, |reply| Command::AwaitHuman {
            tab_id: *tab_id,
            reason,
            timeout,
            reply,
        })
        .await
    }

    // A Lease the person took over goes back to the Client it came from (§21 Flow 1).
    pub async fn human_done(
        &self,
        app: &Arc<App>,
        instance_id: &InstanceId,
        tab_id: &TabId,
    ) -> Result<Tab, Error> {
        let tab = self
            .call(instance_id, |reply| Command::HumanDone {
                tab_id: *tab_id,
                reply,
            })
            .await?;
        app.leases.hand_back_after_done(app, tab_id).await?;

        Ok(tab)
    }

    pub async fn profile_changed(&self, profile_id: &ProfileId, pinned: bool, idle: IdleOverrides) {
        let owners: Vec<mpsc::Sender<Command>> = self
            .owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(|handle| handle.view.borrow().instance.profile_id == Some(*profile_id))
            .map(|handle| handle.commands.clone())
            .collect();

        for commands in owners {
            // An owner that ended meanwhile has nothing left to change.
            let _ = commands
                .send(Command::ProfileChanged {
                    pinned,
                    idle: idle.clone(),
                })
                .await;
        }
    }

    pub async fn set_pool_member(&self, instance_id: &InstanceId, member: bool) {
        if let Some(commands) = self.commands(instance_id) {
            // An owner that ended meanwhile has nothing left to spare.
            let _ = commands.send(Command::PoolMember { member }).await;
        }
    }

    pub async fn recompute_idle(&self) {
        let owners: Vec<mpsc::Sender<Command>> = self
            .owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(|handle| handle.commands.clone())
            .collect();

        for commands in owners {
            let _ = commands.send(Command::Recompute).await;
        }
    }

    // Never blocks: it runs on the path of every mutating tool. A full queue already tells
    // the owner that activity is fresh.
    pub fn activity(&self, instance_id: &InstanceId, tab_id: Option<&TabId>) {
        let owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);

        if let Some(handle) = owners.get(instance_id) {
            let _ = handle.activity.try_send(Activity {
                tab_id: tab_id.copied(),
                at: now_ms(),
            });
        }
    }

    pub async fn activate_tab(
        &self,
        instance_id: &InstanceId,
        tab_id: &TabId,
    ) -> Result<(), Error> {
        self.call(instance_id, |reply| Command::ActivateTab {
            tab_id: *tab_id,
            reply,
        })
        .await
        .map(|_| ())
    }

    pub async fn navigate(
        &self,
        instance_id: &InstanceId,
        tab_id: &TabId,
        url: String,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<(Tab, Navigated), Error> {
        let (navigated, outcome) = oneshot::channel();
        let tab = self
            .call(instance_id, |reply| Command::Navigate {
                tab_id: *tab_id,
                url,
                wait_until,
                timeout,
                navigated,
                reply,
            })
            .await?;
        let outcome = outcome
            .await
            .map_err(|_| internal("the navigation answered without its outcome"))?;

        Ok((tab, outcome))
    }

    // The owner may have ended meanwhile: then there is no Tab left to update.
    pub async fn tab_info(
        &self,
        instance_id: &InstanceId,
        tab_id: &TabId,
        url: String,
        title: String,
    ) {
        if let Some(commands) = self.commands(instance_id) {
            let _ = commands
                .send(Command::TabInfo {
                    tab_id: *tab_id,
                    url,
                    title,
                })
                .await;
        }
    }

    pub async fn cookies(&self, instance_id: &InstanceId) -> Result<Vec<Cookie>, Error> {
        self.call(instance_id, |reply| Command::Cookies { reply })
            .await
    }

    pub async fn set_cookies(
        &self,
        instance_id: &InstanceId,
        cookies: Vec<Cookie>,
    ) -> Result<u32, Error> {
        self.call(instance_id, |reply| Command::SetCookies { cookies, reply })
            .await
    }

    pub fn loaded_extensions(&self) -> Vec<PathBuf> {
        let owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);

        owners
            .values()
            .flat_map(|handle| handle.extensions.iter().cloned())
            .collect()
    }

    // An owned clone: no lock is held across the Page calls that follow.
    // A Live Tab is attached on its first use, which is this call.
    pub async fn page(&self, instance_id: &InstanceId, tab_id: &TabId) -> Result<Page, Error> {
        let not_open = || Error::NotFound {
            what: format!("open tab {tab_id}"),
        };
        let (driver, target_id) = {
            let owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);
            let handle = owners.get(instance_id).ok_or_else(not_open)?;
            let target_id = handle
                .view
                .borrow()
                .tabs
                .iter()
                .find(|tab| tab.id == *tab_id && tab.state != TabState::Closed)
                .map(|tab| tab.target_id.clone())
                .ok_or_else(not_open)?;
            let driver = handle.driver.borrow().clone().ok_or_else(not_open)?;

            (driver, target_id)
        };

        driver.attach_tab(&target_id).await?;

        driver.page(&target_id)
    }

    pub async fn downstream(
        &self,
        instance_id: &InstanceId,
    ) -> Result<(Downstream, mpsc::Receiver<Outbound>), Error> {
        let driver = {
            let owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);

            owners
                .get(instance_id)
                .and_then(|handle| handle.driver.borrow().clone())
        }
        .ok_or_else(|| Error::NotFound {
            what: format!("running instance {instance_id}"),
        })?;

        driver.downstream().await
    }

    pub fn view(&self, instance_id: &InstanceId) -> Option<InstanceView> {
        let owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);

        owners
            .get(instance_id)
            .map(|handle| handle.view.borrow().clone())
    }

    pub fn tab_awaits_human(&self, instance_id: &InstanceId, tab_id: &TabId) -> bool {
        self.view(instance_id).is_some_and(|view| {
            view.tabs
                .iter()
                .any(|tab| tab.id == *tab_id && tab.state == TabState::AwaitingHuman)
        })
    }

    pub fn views(&self) -> Vec<InstanceView> {
        let owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);

        owners
            .values()
            .map(|handle| handle.view.borrow().clone())
            .collect()
    }

    // A launch that slipped past the gate registers before it can be missed, so the rounds
    // repeat until nothing is left.
    pub async fn shutdown_all(&self, app: &Arc<App>) {
        self.closed.store(true, AtomicOrdering::SeqCst);
        // The longest stop any run may take: a Host's persistent run also sends its Profile back.
        let grace = self
            .views()
            .iter()
            .map(|view| owner::copy_back_grace(&view.instance))
            .max()
            .unwrap_or_default();
        let wait =
            Duration::from_secs(app.settings().instance.stop_timeout_s) + grace + SHUTDOWN_MARGIN;

        let rounds = async {
            loop {
                let (live, mut managed): (Vec<_>, Vec<_>) = self
                    .views()
                    .into_iter()
                    .map(|view| (view.instance.kind, view.instance.id))
                    .partition(|(kind, _)| *kind == InstanceKind::Live);
                managed.extend(
                    self.launching_ids()
                        .into_iter()
                        .map(|id| (InstanceKind::Managed, id)),
                );
                if managed.is_empty() && live.is_empty() {
                    return;
                }

                // A Live browser is never stopped, only let go of.
                let stops = join_all(
                    managed
                        .iter()
                        .map(|(_, id)| self.stop(app, id, EndReason::Shutdown)),
                );
                let detaches = join_all(
                    live.iter()
                        .map(|(_, id)| self.detach_with(app, id, EndReason::Shutdown)),
                );
                let (stopped, detached) = futures::future::join(stops, detaches).await;
                for error in stopped.into_iter().chain(detached).filter_map(Result::err) {
                    warn!(%error, "instance not stopped at shutdown");
                }
            }
        };
        if time::timeout(wait, rounds).await.is_err() {
            warn!("instances still running after the shutdown wait");
        }
    }

    fn ensure_open(&self) -> Result<(), Error> {
        if self.closed.load(AtomicOrdering::SeqCst) {
            return Err(Error::DependencyMissing {
                name: "daemon".to_owned(),
                help: "the daemon is shutting down; retry once it is back".to_owned(),
            });
        }

        Ok(())
    }

    fn begin_launch(&self, instance_id: InstanceId) -> (Launching<'_>, Cancel) {
        let (signal, cancel) = watch::channel(None);
        self.launching
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(instance_id, signal);

        (
            Launching {
                instances: self,
                instance_id,
            },
            cancel,
        )
    }

    // The events are subscribed before the signal, so the launch's end cannot be missed.
    fn cancel_launch(
        &self,
        app: &Arc<App>,
        instance_id: &InstanceId,
        reason: EndReason,
    ) -> Option<broadcast::Receiver<Arc<Published>>> {
        let launching = self
            .launching
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let signal = launching.get(instance_id)?;
        let ended = app.events.subscribe();
        signal.send_replace(Some(reason));

        Some(ended)
    }

    fn launching_ids(&self) -> Vec<InstanceId> {
        self.launching
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .copied()
            .collect()
    }

    pub(super) fn forget(&self, instance_id: &InstanceId) {
        self.owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(instance_id);
    }

    fn commands(&self, instance_id: &InstanceId) -> Option<mpsc::Sender<Command>> {
        let owners = self.owners.lock().unwrap_or_else(PoisonError::into_inner);

        owners
            .get(instance_id)
            .map(|handle| handle.commands.clone())
    }

    async fn call<T>(
        &self,
        instance_id: &InstanceId,
        command: impl FnOnce(Reply<T>) -> Command,
    ) -> Result<T, Error> {
        let commands = self.commands(instance_id).ok_or_else(|| Error::NotFound {
            what: format!("running instance {instance_id}"),
        })?;
        let ended = || internal(format!("instance {instance_id} ended before it answered"));

        let (reply, done) = oneshot::channel();
        commands.send(command(reply)).await.map_err(|_| ended())?;

        done.await.map_err(|_| ended())?
    }
}

// The URL the user typed minus anything that could carry a credential.
fn origin(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();

    format!("{scheme}://{host}")
}

async fn connect(
    app: &Arc<App>,
    source: AttachSource,
    instance_id: InstanceId,
) -> Result<brr_engine::Attached, Error> {
    let connection = match source {
        AttachSource::CdpUrl(url) => {
            Connection::WebSocket(brr_cdp::resolve_http_endpoint(&app.http, &url).await?)
        }
        AttachSource::InstallName(name) => {
            let config_root = app
                .paths
                .config_dir
                .parent()
                .unwrap_or(&app.paths.config_dir);

            Connection::WebSocket(devtools_active_port(config_root, &name)?)
        }
        AttachSource::Extension(stream) => Connection::Relay(stream),
    };

    Driver::attach(AttachSpec {
        instance_id,
        connection,
    })
    .await
}

async fn fail_attach(app: &Arc<App>, instance: &mut Instance, error: &Error) {
    instance.ended_at = Some(now_ms());
    instance.end_reason = Some(EndReason::LaunchError);
    instance.error = Some(error.to_string());

    let trigger = InstanceTrigger::AttachError;
    if let Err(error) = transition(app, instance, trigger, Some(error.to_string())).await {
        warn!(instance_id = %instance.id, %error, "failed attach not recorded");
    }
}

// The system Build of the same browser version, if one was detected.
async fn matching_build(app: &Arc<App>, browser_version: &str) -> Option<BuildId> {
    let version = browser_version
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    let builds = app.store.run(|c| build::all(c)).await.ok()?;

    builds
        .into_iter()
        .find(|build| build.source == BuildSource::System && build.version == version)
        .map(|build| build.id)
}

// The owner is shared with launched browsers; for a Live one nothing is launched, so only the
// fields it reads are real.
fn live_spec(app: &Arc<App>, instance_id: InstanceId) -> LaunchSpec {
    LaunchSpec {
        instance_id,
        engine: Engine::Chromium,
        executable: PathBuf::new(),
        user_data_dir: PathBuf::new(),
        log_file: app.paths.instance_log(&instance_id),
        display: Display::Visible,
        display_env: app.display_env.clone(),
        settings: ProfileSettings::default(),
        start_timeout: Duration::from_secs(app.settings().instance.start_timeout_s),
        extensions: Vec::new(),
        proxy_server: None,
        download_dir: app.paths.downloads_dir().join(instance_id.to_string()),
        console_buffer: app.settings().tab.console_buffer,
        network_buffer: app.settings().tab.network_buffer,
        camoufox: None,
        sandbox: None,
    }
}

// The launch task applies the cancel; this returns the row it leaves.
async fn wait_final(
    app: &Arc<App>,
    instance_id: &InstanceId,
    mut ended: broadcast::Receiver<Arc<Published>>,
) -> Result<Instance, Error> {
    let id = *instance_id;
    let wait = async {
        // The launch may have ended before the subscription: the row says so.
        match app.store.run(move |c| instance::get(c, &id)).await {
            Ok(row) if row.state.is_final() => return Ok(row),
            // Not recorded yet: its end is announced like any other.
            Ok(_) | Err(Error::NotFound { .. }) => {}
            Err(error) => return Err(error),
        }

        loop {
            match ended.recv().await {
                Ok(published) if is_final_change(&published, instance_id) => break,
                // Events were dropped: the row says whether this one was among them.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let row = app.store.run(move |c| instance::get(c, &id)).await?;
                    if row.state.is_final() {
                        return Ok(row);
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Ok(_) => {}
            }
        }

        app.store.run(move |c| instance::get(c, &id)).await
    };
    let limit = Duration::from_secs(app.settings().instance.start_timeout_s) + CANCEL_WAIT;

    time::timeout(limit, wait).await.map_err(|_| {
        internal(format!(
            "instance {instance_id} did not end after its cancel"
        ))
    })?
}

fn is_final_change(published: &Published, instance_id: &InstanceId) -> bool {
    let event = &published.event;

    event.kind == STATE_CHANGED
        && event.data["instance_id"] == instance_id.to_string()
        && serde_json::from_value::<InstanceState>(event.data["to"].clone())
            .is_ok_and(InstanceState::is_final)
}

async fn prepare(
    app: &Arc<App>,
    instance_id: &InstanceId,
    profile: &Profile,
    build: &Build,
    sandbox_mode: SandboxMode,
) -> Result<Prepared, Error> {
    let sandbox = match sandbox_mode {
        SandboxMode::Docker => {
            let docker = app.sandbox.target(app).await?;
            let run_dir = app.paths.runs_dir().join(instance_id.to_string());
            DirBuilder::new()
                .recursive(true)
                .mode(RUN_DIR_MODE)
                .create(&run_dir)
                .map_err(internal)?;

            Some(SandboxSpec {
                run_dir,
                docker,
                copy_back: profile.kind == ProfileKind::Persistent,
            })
        }
        SandboxMode::None => None,
    };
    let proxy_server = proxies::for_launch(app, instance_id, profile, sandbox.is_some()).await?;
    let proxy_id = proxies::instance_proxy_id(app, instance_id);
    let (extensions, loaded) = extensions::launch_paths(app, profile, build).await?;
    let extensions = match companion_path(app, profile, build)? {
        Some(path) => [extensions, vec![path]].concat(),
        None => extensions,
    };

    let geo = camoufox::launch_geo(app, instance_id, profile).await?;
    let mut settings = profile.settings.clone();
    let camoufox = if build.engine == Engine::Camoufox {
        Some(camoufox::launch_spec(app, profile, build.id, geo, extensions.clone()).await?)
    } else {
        if let Some(geo) = &geo {
            camoufox::apply_geo(&mut settings, geo);
        }

        None
    };

    Ok(Prepared {
        proxy_server,
        proxy_id,
        extensions,
        loaded,
        settings,
        camoufox,
        sandbox,
    })
}

// A stop that came before the launch asked for a slot: nothing runs yet, the row ends stopped.
async fn end_unadmitted(
    app: &Arc<App>,
    instance: &mut Instance,
    mode: AdmissionMode,
) -> Result<(Instance, Tab), Error> {
    let error = cancelled_launch();
    proxies::release(app, &instance.id);
    sandbox::remove_run_dir(&app.paths, &instance.id);
    fail_admission(app, instance, mode, &error).await;

    Err(error)
}

fn cancelled_launch() -> Error {
    Error::CapExceeded {
        retry_after: None,
        queue_pos: None,
    }
}

// Chromium writes "Last Version" into the data dir; opening a profile with an older browser
// can corrupt it.
fn ensure_no_downgrade(data_dir: &Path, build: &Build) -> Result<(), Error> {
    // Brave's release version (1.x) is not the Chromium version Chromium writes.
    if build.source == BuildSource::Brave {
        return Ok(());
    }
    let Ok(last) = fs::read_to_string(data_dir.join(LAST_VERSION_FILE)) else {
        return Ok(());
    };

    if compare_versions(last.trim(), &build.version) == Ordering::Greater {
        return Err(Error::Conflict {
            reason: ConflictReason::Downgrade,
            detail: None,
        });
    }

    Ok(())
}

async fn start_browser(spec: &LaunchSpec, persistent: bool) -> Result<brr_engine::Launched, Error> {
    if persistent {
        for file in SINGLETON_FILES {
            match fs::remove_file(spec.user_data_dir.join(file)) {
                Err(error) if error.kind() != ErrorKind::NotFound => return Err(internal(error)),
                _ => {}
            }
        }
    } else {
        fs::create_dir_all(&spec.user_data_dir).map_err(internal)?;
    }

    Driver::launch(spec.clone()).await
}

// Hands a sandboxed run's proxy tunnels to its forwarder until the container's stdio closes.
fn serve_tunnels(app: &Arc<App>, instance_id: InstanceId, tunnels: Option<mpsc::Receiver<Stream>>) {
    let Some(mut tunnels) = tunnels else {
        return;
    };
    let app = Arc::clone(app);
    tokio::spawn(async move {
        while let Some(tunnel) = tunnels.recv().await {
            proxies::serve_tunnel(&app, &instance_id, tunnel);
        }
    });
}

// No retry elsewhere: the error names the Host. Only a failed `docker -H` call says the Host is
// unusable, and placement then skips it until its next probe; a browser or Profile failure
// leaves its status as it was.
fn host_launch_failed(app: &App, host: &Host, error: &Error) -> Error {
    let message = format!("launch on host {} failed: {error}", host.name);
    warn!(host_id = %host.id, %message, "host launch failed");
    if matches!(error, Error::Internal { message: text } if text.starts_with(DOCKER_FAILED_PREFIX))
    {
        app.hosts.launch_failed(host.id, message.clone());
    }

    internal(message)
}

fn closed_while_starting(error: &Error) -> bool {
    matches!(error, Error::Internal { message } if message == CONNECTION_CLOSED || message == BROWSER_CLOSED)
}

// A browser that dies while starting leaves its reason in its log, not on the closed pipe.
fn exited_while_starting(spec: &LaunchSpec) -> Option<Error> {
    let line = last_log_line(&spec.log_file)?;
    if line.contains(NO_USABLE_SANDBOX) {
        return Some(Error::DependencyMissing {
            name: CHROMIUM_SANDBOX.to_owned(),
            help: NO_USABLE_SANDBOX_HELP.to_owned(),
        });
    }
    let hint = if spec.display == Display::Visible {
        VISIBLE_EXIT_HINT.to_owned()
    } else {
        format!("see {} for the whole log", spec.log_file.display())
    };

    Some(internal(format!(
        "the browser exited while starting: {line}; {hint}"
    )))
}

// The browser's own text: control characters go and the length is capped.
fn last_log_line(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(LOG_TAIL_BYTES)))
        .ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    let text = String::from_utf8_lossy(&tail);

    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let cause = |level: &str| {
        lines.iter().rev().find(|line| {
            line.contains(level) && !GENERIC_EXITS.iter().any(|generic| line.contains(generic))
        })
    };
    let line = *cause(CHROMIUM_FATAL)
        .or_else(|| cause(CHROMIUM_ERROR))
        .or(lines.last())?;
    let message = match line.split_once(CHROMIUM_PREFIX_END) {
        Some((prefix, message)) if prefix.starts_with('[') => message,
        _ => line,
    };
    let message: String = message
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    Some(
        message
            .trim_end_matches('.')
            .chars()
            .take(LOG_LINE_MAX_CHARS)
            .collect(),
    )
}

// E3: the companion loads into the managed browser like any unpacked extension.
fn companion_path(
    app: &Arc<App>,
    profile: &Profile,
    build: &Build,
) -> Result<Option<PathBuf>, Error> {
    if !profile.settings.companion {
        return Ok(None);
    }
    relay::companion::require_unpacked(build)?;

    relay::companion::companion_dir(app).map(Some)
}
