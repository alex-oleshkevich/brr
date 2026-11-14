pub(crate) mod idle;
mod owner;
pub(crate) mod promote;

use std::{
    cmp::Ordering,
    collections::HashMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    },
    time::Duration,
};

use brr_cdp::{Downstream, Outbound};
use brr_core::{
    AdmissionMode, Build, BuildId, BuildSource, ConflictReason, Cookie, Display, EndReason, Engine,
    Error, EventData, HumanReason, IdleOverrides, Instance, InstanceId, InstanceKind,
    InstanceState, InstanceTrigger, Profile, ProfileId, ProfileKind, ProfileSettings, Tab,
    TabCloseReason, TabId, TabState, WaitUntil, compare_versions, now_ms,
};
use brr_engine::{
    AttachSpec, CamoufoxSpec, Connection, ContextId, Driver, LaunchSpec, devtools_active_port,
    page::Page,
};
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
        proxies, quota, relay,
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
    extensions: Vec<PathBuf>,
    settings: ProfileSettings,
    camoufox: Option<CamoufoxSpec>,
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
            prepared = prepare(app, &instance_id, &profile, &build) => prepared,
            _ = cancelled(&mut cancel) => return end_unadmitted(app, &mut instance, req.admission).await,
        };
        let Prepared {
            proxy_server,
            extensions,
            settings,
            camoufox: camoufox_spec,
        } = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                fail_launch(app, &mut instance, &error).await;

                return Err(error);
            }
        };

        let admitted = app
            .admission
            .admit(
                app,
                AdmitRequest {
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
        if let Err(error) = admitted {
            proxies::release(app, &instance_id);
            fail_admission(app, &mut instance, req.admission, &error).await;

            return Err(error);
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
        };

        // Computed before the process exists: the list wait must not sit between spawning the
        // browser and tracking it.
        let options = app
            .content
            .options(app, spec.settings.block_ads, spec.settings.dismiss_cookies)
            .await;
        let started = time::Instant::now();
        let launched = match start_browser(&spec, persistent).await {
            Ok(launched) => launched,
            Err(error) => {
                fail_launch(app, &mut instance, &error).await;
                app.admission.finished(app, &instance_id).await;

                return Err(error);
            }
        };

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
    ) -> Result<Tab, Error> {
        self.call(instance_id, |reply| Command::Navigate {
            tab_id: *tab_id,
            url,
            wait_until,
            timeout,
            reply,
        })
        .await
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
        let wait = Duration::from_secs(app.settings().instance.stop_timeout_s) + SHUTDOWN_MARGIN;

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
) -> Result<Prepared, Error> {
    let proxy_server = proxies::for_launch(app, instance_id, profile).await?;
    let extensions = extensions::launch_paths(app, profile, build).await?;
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
        extensions,
        settings,
        camoufox,
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
