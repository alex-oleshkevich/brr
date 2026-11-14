use std::{
    collections::{BTreeSet, VecDeque},
    future::pending,
    sync::Arc,
    time::Duration,
};

use brr_core::{
    AdmissionMode, Cookie, EndReason, Error, EventData, HumanReason, IdleOverrides, Instance,
    InstanceKind, InstanceState, InstanceTrigger, ReleaseReason, Tab, TabCloseReason, TabId,
    TabState, TabTrigger, TargetId, WaitUntil, now_ms,
};
use brr_engine::{ContextId, Driver, DriverEvent, LaunchSpec, Launched};
use brr_store::{instance, tab};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::{self, Instant},
};
use tracing::{debug, error, info, warn};

use super::{
    BLANK_PAGE, InstanceView,
    idle::{self, Due, Grace, TabReap, Timeouts},
    promote,
};
use crate::{
    app::App,
    internal,
    services::admission::RETRY_AFTER_S,
    services::{
        downloads::{self, DownloadCtx, Tracker},
        proxies,
        supervisor::remove_ephemeral,
        until,
    },
};

const MAX_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_secs(10 * 60);
const RESTART_BACKOFF_BASE_S: u64 = 2;
const BLANK_TIMEOUT: Duration = Duration::from_secs(10);
// A new Tab must be announced by the browser within this time.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) type Reply<T> = oneshot::Sender<Result<T, Error>>;

pub(super) enum Command {
    OpenTab {
        url: String,
        reply: Reply<Tab>,
    },
    CreateContext {
        reply: Reply<ContextId>,
    },
    OpenTabIn {
        context_id: ContextId,
        url: String,
        reply: Reply<Tab>,
    },
    DisposeContext {
        context_id: ContextId,
        reply: Reply<()>,
    },
    CloseTab {
        tab_id: TabId,
        reason: TabCloseReason,
        reply: Reply<Tab>,
    },
    // The Tab stays, untouched, when it is the only one left: closing it would stop the Instance.
    CloseTabUnlessLast {
        tab_id: TabId,
        reason: TabCloseReason,
        reply: Reply<Tab>,
    },
    SetTabPinned {
        tab_id: TabId,
        pinned: bool,
        reply: Reply<Tab>,
    },
    ProfileChanged {
        pinned: bool,
        idle: IdleOverrides,
    },
    // Settings changed: the loop only has to re-aim its timer, which every message does.
    Recompute,
    // A member of the one-shot pool is spared by the idle reaper, whatever its Profile says.
    PoolMember {
        member: bool,
    },
    Promote {
        name: String,
        reply: Reply<Instance>,
    },
    ActivateTab {
        tab_id: TabId,
        reply: Reply<Tab>,
    },
    Navigate {
        tab_id: TabId,
        url: String,
        wait_until: WaitUntil,
        timeout: Duration,
        reply: Reply<Tab>,
    },
    // Chromium sends no Target.targetInfoChanged for a title the page changed by itself, so a
    // Tab tool reports what it read. A URL that differs from the owner's is a navigation the
    // driver's own events already carry or are about to, so only the title is taken.
    TabInfo {
        tab_id: TabId,
        url: String,
        title: String,
    },
    Cookies {
        reply: Reply<Vec<Cookie>>,
    },
    SetCookies {
        cookies: Vec<Cookie>,
        reply: Reply<u32>,
    },
    Stop {
        reason: EndReason,
        reply: Reply<Instance>,
    },
    // A Live browser is left as it is; only brr's connection to it ends.
    Detach {
        reason: EndReason,
        reply: Reply<Instance>,
    },
    Ready {
        target_id: TargetId,
        reply: Reply<Tab>,
    },
    // The launch saw its first page ready: the Instance runs from here.
    Started {
        reply: Reply<Instance>,
    },
    // The launch could not finish (start timeout, or the Instance could not be recorded as
    // running): the browser goes and the row ends failed.
    StartFailed {
        error: Error,
        reply: Reply<Instance>,
    },
    // The reply parks until the person is done, the wait runs out or the Tab goes away.
    AwaitHuman {
        tab_id: TabId,
        reason: HumanReason,
        timeout: Duration,
        reply: Reply<Tab>,
    },
    HumanDone {
        tab_id: TabId,
        reply: Reply<Tab>,
    },
}

pub(super) struct Inbox {
    pub commands: mpsc::Receiver<Command>,
    pub activity: mpsc::Receiver<Activity>,
}

pub(super) struct Activity {
    pub tab_id: Option<TabId>,
    pub at: i64,
}

#[derive(Clone, Copy)]
enum Until {
    Now,
    Attached,
    Ready,
    Closed,
}

struct Waiter {
    target_id: TargetId,
    until: Until,
    reply: Reply<Tab>,
}

struct HumanWait {
    target_id: TargetId,
    timeout: Duration,
    deadline: Instant,
    reply: Reply<Tab>,
}

// Results of driver calls come back through here: the loop never awaits a driver call itself.
enum Internal {
    Wait(Waiter),
    CloseFailed { target_id: TargetId, error: Error },
    OpenSettled,
    Relaunched(Result<Launched, Error>),
}

struct OwnedTab {
    tab: Tab,
    close_reason: Option<TabCloseReason>,
}

pub(super) struct Owner {
    app: Arc<App>,
    instance: Instance,
    spec: LaunchSpec,
    driver: Option<Driver>,
    // What Instances::page reads: the Driver of the browser that runs now, not the first one.
    published: watch::Sender<Option<Driver>>,
    tabs: Vec<OwnedTab>,
    waiters: Vec<Waiter>,
    human_waits: Vec<HumanWait>,
    stop_reason: EndReason,
    stop_waiters: Vec<Reply<Instance>>,
    pending_stop: Option<EndReason>,
    // The first launch has not seen its first page yet; a relaunch is not this.
    awaiting_start: bool,
    restarts: VecDeque<Instant>,
    kill_at: Option<Instant>,
    restart_at: Option<Instant>,
    // What the reaper looks at: the Profile's pin, or membership of the one-shot pool.
    pinned: bool,
    profile_pinned: bool,
    pool_member: bool,
    opening: usize,
    grace: Grace,
    blanked: BTreeSet<TabId>,
    due_at: Option<Instant>,
    view: watch::Sender<InstanceView>,
    inbox: mpsc::UnboundedSender<Internal>,
    downloads: Arc<Tracker>,
}

pub(crate) async fn transition(
    app: &Arc<App>,
    instance: &mut Instance,
    trigger: InstanceTrigger,
    reason: Option<String>,
) -> Result<(), Error> {
    let from = persist(app, instance, trigger).await?;

    announce(app, instance, from, trigger, reason).await
}

async fn persist(
    app: &Arc<App>,
    instance: &mut Instance,
    trigger: InstanceTrigger,
) -> Result<InstanceState, Error> {
    let from = instance.state;
    let mut row = instance.clone();
    row.state = from.transition(trigger)?;

    // Memory follows the store: a failed write leaves the Instance in the state it recorded.
    let stored = row.clone();
    app.store.run(move |c| instance::update(c, &stored)).await?;
    instance.state = row.state;

    Ok(from)
}

async fn announce(
    app: &Arc<App>,
    instance: &Instance,
    from: InstanceState,
    trigger: InstanceTrigger,
    reason: Option<String>,
) -> Result<(), Error> {
    let to = instance.state;
    app.events
        .publish(EventData::InstanceStateChanged {
            instance_id: instance.id,
            profile_id: instance.profile_id,
            from: Some(from),
            to,
            trigger,
            reason,
        })
        .await?;
    info!(instance_id = %instance.id, ?from, ?to, ?trigger, "instance state changed");

    Ok(())
}

pub(super) async fn fail_launch(app: &Arc<App>, instance: &mut Instance, error: &Error) {
    let (trigger, end_reason) = match error {
        Error::Timeout { .. } => (InstanceTrigger::StartTimeout, EndReason::StartTimeout),
        _ => (InstanceTrigger::LaunchError, EndReason::LaunchError),
    };
    instance.ended_at = Some(now_ms());
    instance.end_reason = Some(end_reason);
    instance.error = Some(error.to_string());

    if let Err(error) = transition(app, instance, trigger, Some(error.to_string())).await {
        warn!(instance_id = %instance.id, %error, "failed launch not recorded");
    }
    sweep_ended(app, instance).await;
}

// A stop that reached a launch before it had an owner: Starting goes down the normal way.
pub(super) async fn stop_unowned(app: &Arc<App>, instance: &mut Instance, reason: EndReason) {
    instance.ended_at = Some(now_ms());
    instance.end_reason = Some(reason);

    for trigger in [InstanceTrigger::Stop, InstanceTrigger::Exited] {
        if let Err(error) = transition(app, instance, trigger, None).await {
            warn!(instance_id = %instance.id, %error, "cancelled launch not recorded");

            break;
        }
    }
    sweep_ended(app, instance).await;
}

// A request that never got a slot ends stopped: waited out, cancelled, or turned away.
pub(super) async fn fail_admission(
    app: &Arc<App>,
    instance: &mut Instance,
    mode: AdmissionMode,
    error: &Error,
) {
    let (trigger, end_reason, reason) = match error {
        Error::CapExceeded {
            retry_after: None, ..
        } => (
            InstanceTrigger::Cancel,
            EndReason::Cancel,
            "cancelled".to_owned(),
        ),
        Error::CapExceeded { .. } if mode == AdmissionMode::Reject => (
            InstanceTrigger::Cancel,
            EndReason::Cancel,
            "rejected".to_owned(),
        ),
        Error::CapExceeded { .. } => (
            InstanceTrigger::QueueTimeout,
            EndReason::QueueTimeout,
            "queue_timeout".to_owned(),
        ),
        _ => (
            InstanceTrigger::Cancel,
            EndReason::Cancel,
            error.to_string(),
        ),
    };
    instance.ended_at = Some(now_ms());
    instance.end_reason = Some(end_reason);
    instance.error = Some(error.to_string());

    if let Err(error) = transition(app, instance, trigger, Some(reason)).await {
        warn!(instance_id = %instance.id, %error, "refused launch not recorded");
    }
}

// Promotion moves the data first, and the deletion never runs after a promote, failed or not.
async fn sweep_ended(app: &Arc<App>, instance: &Instance) {
    let delete_data = match promote::finish_promote(app, instance).await {
        Ok(promoted) => promoted.is_none(),
        Err(error) => {
            error!(instance_id = %instance.id, %error, "promotion failed");
            // Whatever is left must not be where a sweep deletes it.
            promote::set_aside(&app.paths, instance);

            false
        }
    };

    // A crash that restarts is not final: the relaunch reuses the forwarder's URL.
    proxies::release(app, &instance.id);
    sweep(app, instance, delete_data).await;
}

// The tree is killed by its group and by marker; ephemeral data goes once the Instance is final.
async fn sweep(app: &Arc<App>, instance: &Instance, delete_data: bool) {
    app.supervisor
        .kill_instance(&instance.id, instance.pid)
        .await;
    app.supervisor.untrack_xvfb(&instance.id);
    if let Some(pid) = instance.pid {
        app.supervisor.untrack(pid);
    }

    if delete_data
        && instance.ephemeral
        && let Some(dir) = &instance.data_dir
    {
        remove_ephemeral(&app.paths, dir);
    }
}

pub(super) fn exit_error(code: Option<i32>, signal: Option<i32>) -> Option<String> {
    match (code, signal) {
        (Some(0), _) | (None, None) => None,
        (Some(code), _) => Some(format!("browser exited with code {code}")),
        (None, Some(signal)) => Some(format!("browser killed by signal {signal}")),
    }
}

fn answer(waiter: Waiter, tab: &Tab) -> Option<Waiter> {
    let reached = match waiter.until {
        Until::Now => true,
        Until::Attached => tab.state != TabState::Opening,
        Until::Ready => tab.state == TabState::Ready,
        Until::Closed => tab.state == TabState::Closed,
    };

    if reached {
        let _ = waiter.reply.send(Ok(tab.clone()));
    } else if tab.state == TabState::Closed {
        let _ = waiter
            .reply
            .send(Err(internal("the tab closed before it was ready")));
    } else {
        return Some(waiter);
    }

    None
}

fn stop_trigger(state: InstanceState, reason: EndReason) -> InstanceTrigger {
    match (reason, state) {
        (EndReason::Evicted | EndReason::MemoryPressure, InstanceState::Reaping) => {
            InstanceTrigger::GraceOver
        }
        (EndReason::Evicted, _) => InstanceTrigger::Evict,
        (EndReason::MemoryPressure, _) => InstanceTrigger::MemoryPressure,
        (EndReason::Reaped, _) => InstanceTrigger::GraceOver,
        _ => InstanceTrigger::Stop,
    }
}

async fn next_event(events: &mut Option<mpsc::Receiver<DriverEvent>>) -> Option<DriverEvent> {
    match events {
        Some(events) => events.recv().await,
        None => pending().await,
    }
}

pub(super) fn spawn(
    app: Arc<App>,
    instance: Instance,
    spec: LaunchSpec,
    events: mpsc::Receiver<DriverEvent>,
    queues: Inbox,
    view: watch::Sender<InstanceView>,
    published: watch::Sender<Option<Driver>>,
) {
    let pinned = view.borrow().pinned;
    let awaiting_start = instance.state == InstanceState::Starting;
    let driver = published.borrow().clone();
    let (inbox, messages) = mpsc::unbounded_channel();
    let owner = Owner {
        app,
        instance,
        spec,
        driver,
        published,
        tabs: Vec::new(),
        waiters: Vec::new(),
        human_waits: Vec::new(),
        stop_reason: EndReason::Stop,
        stop_waiters: Vec::new(),
        pending_stop: None,
        awaiting_start,
        restarts: VecDeque::new(),
        kill_at: None,
        restart_at: None,
        pinned,
        profile_pinned: pinned,
        pool_member: false,
        opening: 0,
        grace: Grace::default(),
        blanked: BTreeSet::new(),
        due_at: None,
        view,
        inbox,
        downloads: Arc::default(),
    };

    tokio::spawn(owner.run(queues, events, messages));
}

impl Owner {
    async fn run(
        mut self,
        Inbox {
            mut commands,
            mut activity,
        }: Inbox,
        events: mpsc::Receiver<DriverEvent>,
        mut messages: mpsc::UnboundedReceiver<Internal>,
    ) {
        let mut events = Some(events);
        self.publish_view();

        while !self.instance.state.is_final() {
            tokio::select! {
                Some(command) = commands.recv() => {
                    // Activity that preceded this command (the guard) is applied first.
                    self.drain_activity(&mut activity).await;
                    self.on_command(command).await;
                }
                Some(seen) = activity.recv() => self.on_activity(seen).await,
                event = next_event(&mut events) => match event {
                    Some(event) => self.on_event(event).await,
                    None => events = None,
                },
                Some(message) = messages.recv() => {
                    if let Some(relaunched) = self.on_message(message).await {
                        events = Some(relaunched);
                    }
                }
                () = until(self.kill_at) => self.on_kill_timer().await,
                () = until(self.restart_at) => self.on_restart_timer().await,
                () = until(self.human_due()) => self.human_timed_out().await,
                () = until(self.due_at) => {
                    self.drain_activity(&mut activity).await;
                    self.on_due().await;
                }
            }

            self.publish_view();
        }

        // A shutdown waits for the artifacts; otherwise a slow store must not hold the slot.
        if self.stop_reason == EndReason::Shutdown {
            self.downloads.quiesce().await;
            downloads::remove_dir(&self.app.paths, &self.instance.id);
        }

        // Out of the map first: a stop that arrives now reads the final row.
        self.app.instances.forget(&self.instance.id);
        for reply in std::mem::take(&mut self.stop_waiters) {
            let _ = reply.send(Ok(self.instance.clone()));
        }
        self.app
            .admission
            .finished(&self.app, &self.instance.id)
            .await;

        if self.stop_reason != EndReason::Shutdown {
            let (downloads, paths, instance_id) = (
                Arc::clone(&self.downloads),
                self.app.paths.clone(),
                self.instance.id,
            );
            tokio::spawn(async move {
                downloads.quiesce().await;
                downloads::remove_dir(&paths, &instance_id);
            });
        }
    }

    // The view is published before the event: whoever reacts to the row or the event, admission
    // picking an eviction victim among them, must already see the new state.
    async fn transition(
        &mut self,
        trigger: InstanceTrigger,
        reason: Option<String>,
    ) -> Result<(), Error> {
        let from = persist(&self.app, &mut self.instance, trigger).await?;
        self.publish_view();

        announce(&self.app, &self.instance, from, trigger, reason).await
    }

    // For a step that must happen whatever the store says (kill, stop, end): the browser is
    // gone or going, so a write that fails is logged and the move is made in memory; the next
    // start reconciles the row. Only a trigger the table refuses is an error.
    async fn apply(&mut self, trigger: InstanceTrigger) -> Result<(), Error> {
        let from = self.instance.state;
        let to = from.transition(trigger)?;

        match persist(&self.app, &mut self.instance, trigger).await {
            Ok(_) => {}
            Err(error) => {
                error!(instance_id = %self.instance.id, ?trigger, %error, "state not stored");
                self.instance.state = to;
            }
        }
        self.publish_view();

        if let Err(error) = announce(&self.app, &self.instance, from, trigger, None).await {
            warn!(instance_id = %self.instance.id, ?trigger, %error, "state change not announced");
        }

        Ok(())
    }

    fn snapshot(&self) -> InstanceView {
        InstanceView {
            instance: self.instance.clone(),
            tabs: self.tabs.iter().map(|owned| owned.tab.clone()).collect(),
            pinned: self.pinned,
        }
    }

    // After every message: the one timer is re-aimed at the earliest deadline (a Tab closing
    // changes which Tab is the last one, so no command can skip this).
    fn publish_view(&mut self) {
        let view = self.snapshot();
        self.due_at = idle::next_due(&view, &self.grace, &self.timeouts()).map(|(at, _)| {
            let wait = u64::try_from(at - now_ms()).unwrap_or(0);

            Instant::now() + Duration::from_millis(wait)
        });
        self.view.send_replace(view);
    }

    fn timeouts(&self) -> Timeouts {
        idle::timeouts(&self.app.settings().idle, &self.spec.settings.idle)
    }

    fn tab_pos(&self, target_id: &TargetId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|owned| owned.tab.target_id == *target_id)
    }

    fn live_driver(&self) -> Result<Driver, Error> {
        match (&self.driver, self.instance.state) {
            (
                Some(driver),
                InstanceState::Running
                | InstanceState::Idle
                | InstanceState::Reaping
                | InstanceState::Attached,
            ) => Ok(driver.clone()),
            (_, state) => Err(internal(format!(
                "instance {} is {state:?}, not running; open tabs and navigate only while it runs",
                self.instance.id
            ))),
        }
    }

    fn find_tab(&self, tab_id: &TabId) -> Result<&Tab, Error> {
        self.tabs
            .iter()
            .map(|owned| &owned.tab)
            .find(|tab| tab.id == *tab_id)
            .ok_or_else(|| Error::NotFound {
                what: format!("tab {tab_id}"),
            })
    }

    fn target_call(&self, tab_id: &TabId) -> Result<(Driver, TargetId), Error> {
        let driver = self.live_driver()?;

        Ok((driver, self.find_tab(tab_id)?.target_id.clone()))
    }

    fn spawn_call(
        &self,
        reply: Reply<Tab>,
        until: Until,
        call: impl Future<Output = Result<TargetId, Error>> + Send + 'static,
    ) {
        let messages = self.inbox.clone();

        tokio::spawn(async move {
            match call.await {
                Ok(target_id) => {
                    let _ = messages.send(Internal::Wait(Waiter {
                        target_id,
                        until,
                        reply,
                    }));
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            }
        });
    }

    fn wait(&mut self, waiter: Waiter) {
        let found = self
            .tab_pos(&waiter.target_id)
            .map(|pos| self.tabs[pos].tab.clone());

        match found {
            Some(tab) => {
                if let Some(waiter) = answer(waiter, &tab) {
                    self.waiters.push(waiter);
                }
            }
            None if matches!(waiter.until, Until::Attached | Until::Ready) => {
                self.waiters.push(waiter);
            }
            None => {
                let _ = waiter.reply.send(Err(Error::NotFound {
                    what: "tab".to_owned(),
                }));
            }
        }
    }

    fn settle(&mut self, tab: &Tab) {
        for waiter in std::mem::take(&mut self.waiters) {
            if waiter.target_id != tab.target_id {
                self.waiters.push(waiter);
            } else if let Some(waiter) = answer(waiter, tab) {
                self.waiters.push(waiter);
            }
        }
    }

    async fn on_command(&mut self, command: Command) {
        match command {
            Command::OpenTab { url, reply } => self.open_tab(None, url, reply),
            Command::OpenTabIn {
                context_id,
                url,
                reply,
            } => self.open_tab(Some(context_id), url, reply),
            Command::CreateContext { reply } => match self.live_driver() {
                Ok(driver) => {
                    tokio::spawn(async move {
                        let _ = reply.send(driver.create_context().await);
                    });
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            },
            Command::DisposeContext { context_id, reply } => match self.live_driver() {
                Ok(driver) => {
                    tokio::spawn(async move {
                        let _ = reply.send(driver.dispose_context(&context_id).await);
                    });
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            },
            Command::CloseTab {
                tab_id,
                reason,
                reply,
            } => self.close_tab(&tab_id, reason, reply).await,
            Command::CloseTabUnlessLast {
                tab_id,
                reason,
                reply,
            } => {
                let open = self
                    .tabs
                    .iter()
                    .filter(|owned| owned.tab.state != TabState::Closing)
                    .count();
                if open > 1 {
                    self.close_tab(&tab_id, reason, reply).await;
                } else {
                    let _ = reply.send(self.find_tab(&tab_id).cloned());
                }
            }
            Command::SetTabPinned {
                tab_id,
                pinned,
                reply,
            } => {
                let _ = reply.send(self.set_tab_pinned(&tab_id, pinned).await);
            }
            Command::ProfileChanged { pinned, idle } => self.profile_changed(pinned, idle).await,
            Command::Recompute => {}
            Command::PoolMember { member } => {
                self.pool_member = member;
                self.pinned = self.profile_pinned || member;
                if self.pinned && self.instance.state == InstanceState::Reaping {
                    self.wake_instance().await;
                }
            }
            Command::TabInfo { tab_id, url, title } => {
                let stale = self
                    .find_tab(&tab_id)
                    .ok()
                    .filter(|tab| tab.url == url && tab.title != title)
                    .map(|tab| tab.target_id.clone());
                if let Some(target_id) = stale {
                    let info = DriverEvent::TabInfo {
                        target_id,
                        url,
                        title,
                    };

                    self.on_event(info).await;
                }
            }
            Command::Promote { name, reply } => {
                let _ = reply.send(self.promote(name).await);
            }
            Command::ActivateTab { tab_id, reply } => match self.target_call(&tab_id) {
                Ok((driver, target_id)) => {
                    self.spawn_call(reply, Until::Now, async move {
                        driver.activate_tab(&target_id).await?;

                        Ok(target_id)
                    });
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            },
            Command::Navigate {
                tab_id,
                url,
                wait_until,
                timeout,
                reply,
            } => match self.target_call(&tab_id) {
                Ok((driver, target_id)) => {
                    self.spawn_call(reply, Until::Now, async move {
                        driver
                            .navigate(&target_id, &url, wait_until, timeout)
                            .await?;

                        Ok(target_id)
                    });
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            },
            // The driver call runs in its own task: the loop must keep serving other messages.
            Command::Cookies { reply } => match self.live_driver() {
                Ok(driver) => {
                    tokio::spawn(async move {
                        let _ = reply.send(driver.cookies().await);
                    });
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            },
            Command::SetCookies { cookies, reply } => match self.live_driver() {
                Ok(driver) => {
                    tokio::spawn(async move {
                        let _ = reply.send(driver.set_cookies(&cookies).await);
                    });
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            },
            Command::Stop { reason, reply } => self.on_stop(reason, reply).await,
            Command::Detach { reason, reply } => self.on_detach(reason, reply).await,
            Command::Started { reply } => {
                let _ = reply.send(self.on_started().await);
            }
            Command::StartFailed { error, reply } => {
                self.fail_start(&error).await;
                let _ = reply.send(Ok(self.instance.clone()));
            }
            Command::Ready { target_id, reply } => self.wait(Waiter {
                target_id,
                until: Until::Ready,
                reply,
            }),
            Command::AwaitHuman {
                tab_id,
                reason,
                timeout,
                reply,
            } => self.await_human(&tab_id, reason, timeout, reply).await,
            Command::HumanDone { tab_id, reply } => {
                let _ = reply.send(self.human_done(&tab_id).await);
            }
        }
    }

    async fn await_human(
        &mut self,
        tab_id: &TabId,
        reason: HumanReason,
        timeout: Duration,
        reply: Reply<Tab>,
    ) {
        let trigger = match reason {
            HumanReason::Dialog => TabTrigger::DialogBlocking,
            HumanReason::Captcha | HumanReason::Login => TabTrigger::CaptchaNeedsHuman,
        };
        let target_id = match self.find_tab(tab_id) {
            Ok(tab) => tab.target_id.clone(),
            Err(error) => {
                let _ = reply.send(Err(error));

                return;
            }
        };
        if let Err(error) = self.try_step(&target_id, trigger).await {
            let _ = reply.send(Err(error));

            return;
        }

        // Parked before the announcement: the Tab is awaiting now and must never lack its timer.
        self.human_waits.push(HumanWait {
            target_id,
            timeout,
            deadline: Instant::now() + timeout,
            reply,
        });

        let deadline = now_ms() + i64::try_from(timeout.as_millis()).unwrap_or(i64::MAX);
        let announced = self
            .app
            .events
            .publish(EventData::TabAwaitingHuman {
                tab_id: *tab_id,
                instance_id: self.instance.id,
                reason,
                deadline,
            })
            .await;
        if let Err(error) = announced {
            warn!(instance_id = %self.instance.id, %error, "awaiting_human not announced");
        }
    }

    async fn human_done(&mut self, tab_id: &TabId) -> Result<Tab, Error> {
        let target_id = self.find_tab(tab_id)?.target_id.clone();
        self.try_step(&target_id, TabTrigger::HumanDone).await?;

        self.find_tab(tab_id).cloned()
    }

    fn human_due(&self) -> Option<Instant> {
        self.human_waits.iter().map(|wait| wait.deadline).min()
    }

    // The waits come out first: a Tab that cannot step any more must not keep the timer firing.
    async fn human_timed_out(&mut self) {
        let now = Instant::now();
        let (expired, pending) = std::mem::take(&mut self.human_waits)
            .into_iter()
            .partition(|wait| wait.deadline <= now);
        self.human_waits = pending;

        for wait in expired {
            self.step(&wait.target_id, TabTrigger::HumanTimeout).await;
            let after_ms = u64::try_from(wait.timeout.as_millis()).unwrap_or(u64::MAX);
            let _ = wait.reply.send(Err(Error::Timeout { after_ms }));
        }
    }

    // The Tab left awaiting_human by HumanDone, or by closing (the Tab or its Instance).
    fn end_human_waits(&mut self, tab: &Tab, trigger: TabTrigger) {
        for wait in std::mem::take(&mut self.human_waits) {
            if wait.target_id != tab.target_id {
                self.human_waits.push(wait);
            } else if trigger == TabTrigger::HumanDone {
                let _ = wait.reply.send(Ok(tab.clone()));
            } else {
                let _ = wait.reply.send(Err(Error::NotFound {
                    what: "open tab".to_owned(),
                }));
            }
        }
    }

    // The per-Instance Tab limit is checked here, where opens are serialized; opens still in
    // flight count, since their Tabs are not listed yet.
    fn open_tab(&mut self, context_id: Option<ContextId>, url: String, reply: Reply<Tab>) {
        let driver = match self.live_driver() {
            Ok(driver) => driver,
            Err(error) => {
                let _ = reply.send(Err(error));

                return;
            }
        };

        // A Tab still opening is the in-flight open itself.
        let listed = self
            .tabs
            .iter()
            .filter(|owned| !matches!(owned.tab.state, TabState::Opening | TabState::Closing))
            // In a Live browser only the Tabs brr opened count: the user's own are not its load.
            .filter(|owned| self.instance.kind == InstanceKind::Managed || owned.tab.opened_by_brr)
            .count();
        let max = self.app.settings().limits.max_tabs_per_instance as usize;
        if listed + self.opening >= max {
            let _ = reply.send(Err(Error::CapExceeded {
                retry_after: Some(RETRY_AFTER_S),
                queue_pos: None,
            }));

            return;
        }

        self.opening += 1;
        let (attached, answered) = oneshot::channel();
        let messages = self.inbox.clone();
        tokio::spawn(async move {
            let result = time::timeout(OPEN_TIMEOUT, answered).await;
            let _ = messages.send(Internal::OpenSettled);
            let _ = match result {
                Ok(Ok(result)) => reply.send(result),
                Ok(Err(_)) => return,
                Err(_) => reply.send(Err(Error::Timeout {
                    after_ms: u64::try_from(OPEN_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
                })),
            };
        });
        self.spawn_call(attached, Until::Attached, async move {
            match context_id {
                Some(context_id) => driver.open_tab_in(&context_id, &url).await,
                None => driver.open_tab(&url).await,
            }
        });
    }

    async fn on_stop(&mut self, reason: EndReason, reply: Reply<Instance>) {
        let trigger = stop_trigger(self.instance.state, reason);

        // Admission and the pressure Reaper race with everything else and must lose cleanly.
        // A Profile pinned since the pick is spared too (I4).
        if matches!(reason, EndReason::Evicted | EndReason::MemoryPressure)
            && (self.pinned
                || !matches!(
                    self.instance.state,
                    InstanceState::Idle | InstanceState::Reaping
                ))
        {
            let state = self.instance.state;
            let error =
                state
                    .transition(trigger)
                    .err()
                    .unwrap_or_else(|| Error::InvalidTransition {
                        from: format!("{state:?}").to_lowercase(),
                        to: "stopping".to_owned(),
                    });
            let _ = reply.send(Err(error));

            return;
        }

        match self.instance.state {
            InstanceState::Running | InstanceState::Idle | InstanceState::Reaping => {
                self.stop_now(trigger, reason, reply).await;
            }
            InstanceState::Starting if self.awaiting_start => {
                self.stop_now(trigger, reason, reply).await;
            }
            // A relaunch is in flight; it is stopped as soon as it is up.
            InstanceState::Starting => {
                self.stop_waiters.push(reply);
                self.pending_stop = Some(reason);
            }
            // Waiting out the restart backoff: stopping cancels the restart.
            InstanceState::Crashed => {
                self.stop_waiters.push(reply);
                self.give_up().await;
            }
            // Already going down: this stop returns when the row is final.
            InstanceState::Stopping | InstanceState::Killed => self.stop_waiters.push(reply),
            state => {
                let error = state
                    .transition(InstanceTrigger::Stop)
                    .err()
                    .unwrap_or_else(|| internal(format!("instance cannot stop from {state:?}")));
                let _ = reply.send(Err(error));
            }
        }
    }

    async fn stop_now(
        &mut self,
        trigger: InstanceTrigger,
        reason: EndReason,
        reply: Reply<Instance>,
    ) {
        match self.begin_stop(trigger, reason).await {
            Ok(()) => self.stop_waiters.push(reply),
            Err(error) => {
                let _ = reply.send(Err(error));
            }
        }
    }

    async fn on_started(&mut self) -> Result<Instance, Error> {
        if self.awaiting_start {
            if self.instance.state == InstanceState::Starting {
                self.transition(InstanceTrigger::Ready, None).await?;
            }
            self.awaiting_start = false;
        }

        Ok(self.instance.clone())
    }

    // The first launch ends here: nothing ran yet, so it is a launch failure and the browser,
    // its proxy and its ephemeral data go with it.
    async fn fail_start(&mut self, error: &Error) {
        if !self.awaiting_start {
            return;
        }
        self.awaiting_start = false;
        self.kill_at = None;

        let (trigger, end_reason) = match error {
            Error::Timeout { .. } => (InstanceTrigger::StartTimeout, EndReason::StartTimeout),
            _ => (InstanceTrigger::LaunchError, EndReason::LaunchError),
        };
        self.app
            .supervisor
            .kill_instance(&self.instance.id, self.instance.pid)
            .await;
        self.close_all(ReleaseReason::InstanceStopped).await;

        self.instance.ended_at = Some(now_ms());
        self.instance.end_reason = Some(end_reason);
        self.instance.error = Some(error.to_string());
        if let Err(error) = self.apply(trigger).await {
            warn!(instance_id = %self.instance.id, %error, "failed start not recorded");
        }

        sweep_ended(&self.app, &self.instance).await;
    }

    async fn on_detach(&mut self, reason: EndReason, reply: Reply<Instance>) {
        if self.instance.state != InstanceState::Attached {
            let state = self.instance.state;
            let error = state
                .transition(InstanceTrigger::Detach)
                .err()
                .unwrap_or_else(|| internal(format!("instance cannot detach from {state:?}")));
            let _ = reply.send(Err(error));

            return;
        }

        if let Some(driver) = self.driver.take() {
            if let Err(error) = driver.detach().await {
                warn!(instance_id = %self.instance.id, %error, "detach not clean");
            }
            self.published.send_replace(None);
        }
        let release = match reason {
            EndReason::Shutdown => ReleaseReason::DaemonRestart,
            _ => ReleaseReason::Released,
        };
        self.end_live(InstanceTrigger::Detach, reason, None, release)
            .await;

        let _ = reply.send(Ok(self.instance.clone()));
    }

    // The browser itself is never touched: no kill, no close, no process sweep.
    async fn end_live(
        &mut self,
        trigger: InstanceTrigger,
        reason: EndReason,
        error: Option<String>,
        release: ReleaseReason,
    ) {
        self.close_all(release).await;

        self.instance.ended_at = Some(now_ms());
        self.instance.end_reason = Some(reason);
        self.instance.error = error;
        if let Err(error) = self.apply(trigger).await {
            warn!(instance_id = %self.instance.id, %error, "live end not recorded");
        }
    }

    fn reapable(&self, tab: &Tab) -> bool {
        matches!(tab.state, TabState::Idle | TabState::Reaping)
            && idle::tab_reap(self.tabs.iter().map(|owned| &owned.tab), &tab.id) == TabReap::Reap
    }

    async fn close_tab(&mut self, tab_id: &TabId, reason: TabCloseReason, reply: Reply<Tab>) {
        let found = self.live_driver().and_then(|driver| {
            let tab = self.find_tab(tab_id)?;
            if self.instance.kind == InstanceKind::Live && !tab.opened_by_brr {
                return Err(Error::InvalidArgument {
                    field: "tab_id".to_owned(),
                    message: "brr closes only Tabs it opened in a Live browser".to_owned(),
                });
            }
            // The pressure Reaper decides from a snapshot: a Tab used, pinned or left as the
            // last one since then is not its to close (I2, I4).
            if reason == TabCloseReason::Reaped && !self.reapable(tab) {
                return Err(Error::InvalidTransition {
                    from: format!("{:?}", tab.state).to_lowercase(),
                    to: "closing".to_owned(),
                });
            }
            tab.state.transition(TabTrigger::Close)?;

            Ok((driver, tab.target_id.clone()))
        });
        let (driver, target_id) = match found {
            Ok(found) => found,
            Err(error) => {
                let _ = reply.send(Err(error));

                return;
            }
        };

        self.waiters.push(Waiter {
            target_id: target_id.clone(),
            until: Until::Closed,
            reply,
        });
        self.begin_close(&target_id, TabTrigger::Close, reason)
            .await;

        // The browser is closed instead of the tab: the exit follows (I2).
        if self.closed_last_tab().await {
            return;
        }

        self.spawn_close(driver, target_id);
    }

    fn spawn_close(&self, driver: Driver, target_id: TargetId) {
        let messages = self.inbox.clone();

        tokio::spawn(async move {
            if let Err(error) = driver.close_tab(&target_id).await {
                let _ = messages.send(Internal::CloseFailed { target_id, error });
            }
        });
    }

    async fn on_message(&mut self, message: Internal) -> Option<mpsc::Receiver<DriverEvent>> {
        match message {
            Internal::Wait(waiter) => self.wait(waiter),
            Internal::OpenSettled => {
                self.opening = self.opening.saturating_sub(1);
                // A wait nobody listens to any more: its open timed out.
                self.waiters.retain(|waiter| !waiter.reply.is_closed());
            }
            Internal::CloseFailed { target_id, error } => {
                for waiter in std::mem::take(&mut self.waiters) {
                    if waiter.target_id == target_id && matches!(waiter.until, Until::Closed) {
                        let _ = waiter.reply.send(Err(error.clone()));
                    } else {
                        self.waiters.push(waiter);
                    }
                }
            }
            Internal::Relaunched(result) => return self.relaunched(result).await,
        }

        None
    }

    async fn on_event(&mut self, event: DriverEvent) {
        match event {
            DriverEvent::TabCreated { target_id, url } => {
                if self.tab_pos(&target_id).is_none()
                    && let Err(error) = self.open_entry(target_id, url).await
                {
                    warn!(instance_id = %self.instance.id, %error, "new tab not recorded");
                }
            }
            DriverEvent::TabAttached { target_id } => {
                self.step(&target_id, TabTrigger::TargetAttached).await;
            }
            DriverEvent::TabLoading { target_id } => {
                self.step(&target_id, TabTrigger::Navigation).await;
            }
            DriverEvent::TabLoaded { target_id } => {
                self.step(&target_id, TabTrigger::Loaded).await;
            }
            DriverEvent::TabInfo {
                target_id,
                url,
                title,
            } => {
                let Some(pos) = self.tab_pos(&target_id) else {
                    return;
                };
                let owned = &mut self.tabs[pos].tab;
                owned.url.clone_from(&url);
                owned.title.clone_from(&title);

                let published = self
                    .app
                    .events
                    .publish(EventData::TabNavigated {
                        tab_id: owned.id,
                        instance_id: self.instance.id,
                        url,
                        title,
                    })
                    .await;
                if let Err(error) = published {
                    warn!(instance_id = %self.instance.id, %error, "navigation not published");
                }
            }
            DriverEvent::TabCrashed { target_id } => {
                self.step(&target_id, TabTrigger::RendererCrash).await;
            }
            DriverEvent::TabDestroyed { target_id } => self.destroyed(&target_id).await,
            DriverEvent::DownloadStarted { .. } | DriverEvent::DownloadProgress { .. } => {
                self.download(event);
            }
            DriverEvent::Exited { code, signal } => self.exited(code, signal).await,
        }
    }

    fn download(&self, event: DriverEvent) {
        let Some(driver) = self.driver.clone() else {
            return;
        };
        let tab = match &event {
            DriverEvent::DownloadStarted {
                target_id: Some(target_id),
                ..
            } => self.tab_pos(target_id).map(|pos| &self.tabs[pos]),
            _ => None,
        };
        let ctx = DownloadCtx {
            profile_id: self.instance.profile_id,
            instance_id: self.instance.id,
            // A page that no Tab owns yet still reports to the Instance's first Tab.
            tab_id: tab.or_else(|| self.tabs.first()).map(|owned| owned.tab.id),
            dir: self.spec.download_dir.clone(),
            limit_mb: self.spec.settings.download_max_mb,
            driver,
        };

        downloads::handle(&self.app, &self.downloads, ctx, event);
    }

    async fn open_entry(&mut self, target_id: TargetId, url: String) -> Result<(), Error> {
        let now = now_ms();
        let opened_by_brr = self
            .driver
            .as_ref()
            .is_none_or(|driver| driver.opened_by_brr(&target_id));
        let opened = Tab {
            id: TabId::new(),
            instance_id: self.instance.id,
            target_id,
            state: TabState::Opening,
            url,
            title: String::new(),
            opened_by_brr,
            pinned: false,
            last_activity_at: now,
            created_at: now,
            closed_at: None,
            lease: None,
        };

        let row = opened.clone();
        self.app.store.run(move |c| tab::insert(c, &row)).await?;
        self.app
            .events
            .publish(EventData::TabOpened {
                tab_id: opened.id,
                instance_id: opened.instance_id,
                url: opened.url.clone(),
                opened_by_brr: opened.opened_by_brr,
            })
            .await?;
        self.app
            .events
            .publish(EventData::TabStateChanged {
                tab_id: opened.id,
                instance_id: opened.instance_id,
                from: None,
                to: TabState::Opening,
                trigger: TabTrigger::Open,
            })
            .await?;

        self.settle(&opened);
        self.tabs.push(OwnedTab {
            tab: opened,
            close_reason: None,
        });

        Ok(())
    }

    // An invalid transition is a race between events (a duplicate load, a late attach) and is
    // not an error worth more than a debug line.
    async fn step(&mut self, target_id: &TargetId, trigger: TabTrigger) {
        match self.try_step(target_id, trigger).await {
            Ok(()) => {}
            Err(Error::InvalidTransition { from, to }) => {
                debug!(instance_id = %self.instance.id, from, to, ?trigger, "tab event ignored");
            }
            Err(error) => {
                warn!(instance_id = %self.instance.id, %error, ?trigger, "tab transition failed");
            }
        }
    }

    async fn try_step(&mut self, target_id: &TargetId, trigger: TabTrigger) -> Result<(), Error> {
        let Some(pos) = self.tab_pos(target_id) else {
            return Ok(());
        };

        let owned = &mut self.tabs[pos];
        let from = owned.tab.state;
        let to = from.transition(trigger)?;
        owned.tab.state = to;
        if to == TabState::Closed {
            owned.tab.closed_at = Some(now_ms());
        }
        let tab = owned.tab.clone();
        let reason = owned.close_reason.unwrap_or(TabCloseReason::Page);

        if to.is_persisted() {
            let row = tab.clone();
            self.app.store.run(move |c| tab::update(c, &row)).await?;
        }

        self.app
            .events
            .publish(EventData::TabStateChanged {
                tab_id: tab.id,
                instance_id: tab.instance_id,
                from: Some(from),
                to,
                trigger,
            })
            .await?;

        match to {
            TabState::Crashed => {
                self.app
                    .events
                    .publish(EventData::TabCrashed {
                        tab_id: tab.id,
                        instance_id: tab.instance_id,
                    })
                    .await?;
            }
            TabState::Closed => {
                self.tabs.remove(pos);
                self.grace.tabs.remove(&tab.id);
                self.blanked.remove(&tab.id);
                self.app
                    .events
                    .publish(EventData::TabClosed {
                        tab_id: tab.id,
                        instance_id: tab.instance_id,
                        reason,
                    })
                    .await?;
                self.app
                    .leases
                    .release_tab(&self.app, &tab.id, ReleaseReason::TabClosed)
                    .await?;
            }
            _ => {}
        }

        if from == TabState::AwaitingHuman {
            self.end_human_waits(&tab, trigger);
        }
        self.settle(&tab);

        Ok(())
    }

    async fn begin_close(
        &mut self,
        target_id: &TargetId,
        trigger: TabTrigger,
        reason: TabCloseReason,
    ) {
        if let Some(pos) = self.tab_pos(target_id)
            && self.tabs[pos].tab.state != TabState::Closing
        {
            self.tabs[pos].close_reason = Some(reason);
            self.step(target_id, trigger).await;
        }
    }

    async fn destroyed(&mut self, target_id: &TargetId) {
        self.begin_close(target_id, TabTrigger::Close, TabCloseReason::Page)
            .await;
        self.step(target_id, TabTrigger::TargetDestroyed).await;
        self.closed_last_tab().await;
    }

    // I2: the browser exit that follows its last Tab closing is a stop, never a crash.
    async fn closed_last_tab(&mut self) -> bool {
        let open = self
            .tabs
            .iter()
            .filter(|owned| owned.tab.state != TabState::Closing)
            .count();
        if open > 0 || self.instance.state != InstanceState::Running {
            return false;
        }

        let begun = self
            .begin_stop(InstanceTrigger::LastTabClosed, EndReason::LastTabClosed)
            .await;
        if let Err(error) = &begun {
            warn!(instance_id = %self.instance.id, %error, "last tab close not applied");
        }

        begun.is_ok()
    }

    async fn begin_stop(
        &mut self,
        trigger: InstanceTrigger,
        reason: EndReason,
    ) -> Result<(), Error> {
        self.apply(trigger).await?;
        self.awaiting_start = false;
        self.stop_reason = reason;

        let targets: Vec<TargetId> = self
            .tabs
            .iter()
            .map(|owned| owned.tab.target_id.clone())
            .collect();
        for target_id in targets {
            self.begin_close(
                &target_id,
                TabTrigger::InstanceStopping,
                TabCloseReason::InstanceStopped,
            )
            .await;
        }

        if let Some(driver) = self.driver.clone() {
            tokio::spawn(async move {
                let _ = driver.close().await;
            });
        }
        let stop_timeout = Duration::from_secs(self.app.settings().instance.stop_timeout_s);
        self.kill_at = Some(Instant::now() + stop_timeout);

        Ok(())
    }

    // Leases go first, so they carry the Instance's reason and not the Tab's.
    async fn close_all(&mut self, release: ReleaseReason) {
        let released = self
            .app
            .leases
            .release_instance(&self.app, &self.instance.id, release)
            .await;
        if let Err(error) = released {
            warn!(instance_id = %self.instance.id, %error, "leases not released");
        }

        let targets: Vec<TargetId> = self
            .tabs
            .iter()
            .map(|owned| owned.tab.target_id.clone())
            .collect();
        for target_id in targets {
            self.begin_close(
                &target_id,
                TabTrigger::InstanceStopping,
                TabCloseReason::InstanceStopped,
            )
            .await;
            self.step(&target_id, TabTrigger::TargetDestroyed).await;
        }
    }

    fn set_driver(&mut self, driver: Option<Driver>) {
        self.published.send_replace(driver.clone());
        self.driver = driver;
    }

    async fn exited(&mut self, code: Option<i32>, signal: Option<i32>) {
        let error = exit_error(code, signal);
        self.set_driver(None);

        match self.instance.state {
            InstanceState::Stopping => {
                self.finish_stopped(InstanceTrigger::Exited, error).await;
            }
            InstanceState::Running | InstanceState::Idle | InstanceState::Reaping => {
                self.crashed(error).await;
            }
            InstanceState::Attached => {
                let lost = Some("the connection to the browser was lost".to_owned());
                self.end_live(
                    InstanceTrigger::ConnectionLost,
                    EndReason::Crashed,
                    lost,
                    ReleaseReason::InstanceStopped,
                )
                .await;
            }
            InstanceState::Starting if self.awaiting_start => {
                let died = error.unwrap_or_else(|| "the browser exited while starting".to_owned());
                self.fail_start(&internal(died)).await;
            }
            state => debug!(instance_id = %self.instance.id, ?state, "browser exit ignored"),
        }
    }

    async fn on_kill_timer(&mut self) {
        self.kill_at = None;

        if let Err(error) = self.apply(InstanceTrigger::StopTimeout).await {
            warn!(instance_id = %self.instance.id, %error, "stop timeout not applied");

            return;
        }

        self.app
            .supervisor
            .kill_instance(&self.instance.id, self.instance.pid)
            .await;
        self.finish_stopped(InstanceTrigger::TreeKilled, None).await;
    }

    async fn finish_stopped(&mut self, trigger: InstanceTrigger, error: Option<String>) {
        self.kill_at = None;
        self.close_all(ReleaseReason::InstanceStopped).await;

        self.instance.ended_at = Some(now_ms());
        self.instance.end_reason = Some(self.stop_reason);
        self.instance.error = error;
        if let Err(error) = self.apply(trigger).await {
            warn!(instance_id = %self.instance.id, %error, "stop not recorded");
        }

        sweep_ended(&self.app, &self.instance).await;
    }

    async fn crashed(&mut self, error: Option<String>) {
        self.close_all(ReleaseReason::InstanceCrashed).await;

        self.instance.error = error;
        if let Err(error) = self.apply(InstanceTrigger::UnexpectedExit).await {
            warn!(instance_id = %self.instance.id, %error, "crash not recorded");
        }
        sweep(&self.app, &self.instance, false).await;
        // The group of this browser is swept; a later sweep must not signal a pid that may be reused.
        self.instance.pid = None;

        while self
            .restarts
            .front()
            .is_some_and(|at| at.elapsed() > RESTART_WINDOW)
        {
            self.restarts.pop_front();
        }

        if self.app.settings().instance.auto_restart && self.restarts.len() < MAX_RESTARTS {
            let backoff = Duration::from_secs(RESTART_BACKOFF_BASE_S << self.restarts.len());
            self.restarts.push_back(Instant::now());
            self.restart_at = Some(Instant::now() + backoff);
        } else {
            self.give_up().await;
        }
    }

    async fn give_up(&mut self) {
        self.restart_at = None;
        self.instance.ended_at = Some(now_ms());
        self.instance.end_reason = Some(EndReason::Crashed);

        if let Err(error) = self.apply(InstanceTrigger::NoRestart).await {
            warn!(instance_id = %self.instance.id, %error, "crash end not recorded");
        }
        sweep_ended(&self.app, &self.instance).await;
    }

    async fn on_restart_timer(&mut self) {
        self.restart_at = None;
        self.instance.restart_count += 1;

        if let Err(error) = self.apply(InstanceTrigger::AutoRestart).await {
            warn!(instance_id = %self.instance.id, %error, "restart not applied");

            return;
        }

        let spec = self.spec.clone();
        let messages = self.inbox.clone();
        tokio::spawn(async move {
            let _ = messages.send(Internal::Relaunched(Driver::launch(spec).await));
        });
    }

    async fn drain_activity(&mut self, activity: &mut mpsc::Receiver<Activity>) {
        while let Ok(seen) = activity.try_recv() {
            self.on_activity(seen).await;
        }
    }

    async fn on_activity(&mut self, seen: Activity) {
        self.instance.last_activity_at = self.instance.last_activity_at.max(seen.at);
        if matches!(
            self.instance.state,
            InstanceState::Idle | InstanceState::Reaping
        ) {
            self.wake_instance().await;
        }

        let Some(pos) = self
            .tabs
            .iter()
            .position(|owned| Some(owned.tab.id) == seen.tab_id)
        else {
            return;
        };
        let tab = &mut self.tabs[pos].tab;
        tab.last_activity_at = tab.last_activity_at.max(seen.at);
        if matches!(tab.state, TabState::Idle | TabState::Reaping) {
            let (tab_id, target_id) = (tab.id, tab.target_id.clone());
            self.wake_tab(&tab_id, &target_id).await;
        }
    }

    async fn wake_instance(&mut self) {
        self.grace.instance = None;

        let applied = self.transition(InstanceTrigger::Activity, None).await;
        if let Err(error) = applied {
            warn!(instance_id = %self.instance.id, %error, "activity not applied");
        }
    }

    async fn wake_tab(&mut self, tab_id: &TabId, target_id: &TargetId) {
        self.grace.tabs.remove(tab_id);
        self.blanked.remove(tab_id);
        self.step(target_id, TabTrigger::Activity).await;
    }

    async fn on_due(&mut self) {
        let due = idle::next_due(&self.snapshot(), &self.grace, &self.timeouts());
        let Some((at, due)) = due.filter(|(at, _)| *at <= now_ms()) else {
            return;
        };
        debug!(instance_id = %self.instance.id, at, ?due, "idle deadline reached");

        match due {
            Due::InstanceIdle => {
                let idled = self.transition(InstanceTrigger::IdleTimeout, None).await;
                match idled {
                    Ok(()) => self.reap_idle_instance().await,
                    Err(error) => {
                        warn!(instance_id = %self.instance.id, %error, "idle not applied");
                    }
                }
            }
            Due::InstanceGraceOver => {
                self.grace.instance = None;
                if let Err(error) = self
                    .begin_stop(InstanceTrigger::GraceOver, EndReason::Reaped)
                    .await
                {
                    warn!(instance_id = %self.instance.id, %error, "reap not applied");
                }
            }
            Due::TabIdle(tab_id) => {
                if let Some(target_id) = self.target_of(&tab_id) {
                    self.step(&target_id, TabTrigger::IdleTimeout).await;
                    self.reap_idle_tab(&tab_id).await;
                }
            }
            Due::TabGraceOver(tab_id) => self.tab_grace_over(&tab_id).await,
        }
    }

    fn target_of(&self, tab_id: &TabId) -> Option<TargetId> {
        self.find_tab(tab_id).ok().map(|tab| tab.target_id.clone())
    }

    async fn reap_idle_instance(&mut self) {
        if self.instance.state != InstanceState::Idle
            || !idle::may_reap_instance(&self.instance, self.pinned)
        {
            return;
        }

        let reaped = self.transition(InstanceTrigger::Reap, None).await;
        if let Err(error) = reaped {
            warn!(instance_id = %self.instance.id, %error, "reap not applied");

            return;
        }

        let grace_ends_at = now_ms() + self.timeouts().grace_ms;
        self.grace.instance = Some(grace_ends_at);
        let warned = self
            .app
            .events
            .publish(EventData::InstanceReapingWarning {
                instance_id: self.instance.id,
                grace_ends_at,
            })
            .await;
        if let Err(error) = warned {
            warn!(instance_id = %self.instance.id, %error, "reaping warning not published");
        }
    }

    async fn reap_idle_tab(&mut self, tab_id: &TabId) {
        let Some(target_id) = self.target_of(tab_id) else {
            return;
        };
        if self.find_tab(tab_id).map(|tab| tab.state) != Ok(TabState::Idle) {
            return;
        }

        match idle::tab_reap(self.tabs.iter().map(|owned| &owned.tab), tab_id) {
            TabReap::Reap => {
                self.step(&target_id, TabTrigger::Reap).await;

                let grace_ends_at = now_ms() + self.timeouts().grace_ms;
                self.grace.tabs.insert(*tab_id, grace_ends_at);
                let warned = self
                    .app
                    .events
                    .publish(EventData::TabReapingWarning {
                        tab_id: *tab_id,
                        instance_id: self.instance.id,
                        grace_ends_at,
                    })
                    .await;
                if let Err(error) = warned {
                    warn!(instance_id = %self.instance.id, %error, "reaping warning not published");
                }
            }
            TabReap::Blank => self.blank(tab_id, target_id),
            TabReap::Keep => {}
        }
    }

    // Once per idle period, so the blank page's own load events cannot start it again.
    fn blank(&mut self, tab_id: &TabId, target_id: TargetId) {
        if !self.blanked.insert(*tab_id) {
            return;
        }
        let Ok(driver) = self.live_driver() else {
            return;
        };

        let instance_id = self.instance.id;
        tokio::spawn(async move {
            let blanked = driver
                .navigate(&target_id, BLANK_PAGE, WaitUntil::Load, BLANK_TIMEOUT)
                .await;
            if let Err(error) = blanked {
                warn!(%instance_id, %error, "last tab not blanked");
            }
        });
    }

    // I2 wins over a grace that began while other Tabs still existed.
    async fn tab_grace_over(&mut self, tab_id: &TabId) {
        let (Some(target_id), Ok(driver)) = (self.target_of(tab_id), self.live_driver()) else {
            return;
        };

        let reap = idle::tab_reap(self.tabs.iter().map(|owned| &owned.tab), tab_id);
        if reap == TabReap::Reap {
            self.begin_close(&target_id, TabTrigger::GraceOver, TabCloseReason::Reaped)
                .await;
            self.spawn_close(driver, target_id);

            return;
        }

        self.wake_tab(tab_id, &target_id).await;
        if reap == TabReap::Blank {
            self.blank(tab_id, target_id);
        }
    }

    async fn set_tab_pinned(&mut self, tab_id: &TabId, pinned: bool) -> Result<Tab, Error> {
        let open = |owned: &OwnedTab| {
            owned.tab.id == *tab_id && !matches!(owned.tab.state, TabState::Closing)
        };
        let Some(pos) = self.tabs.iter().position(open) else {
            return Err(Error::NotFound {
                what: format!("open tab {tab_id}"),
            });
        };
        let target_id = self.tabs[pos].tab.target_id.clone();
        self.tabs[pos].tab.pinned = pinned;

        let row = self.tabs[pos].tab.clone();
        self.app.store.run(move |c| tab::update(c, &row)).await?;

        // Pinning is not activity, but a pinned Tab never stays in reaping.
        if pinned && self.tabs[pos].tab.state == TabState::Reaping {
            self.wake_tab(tab_id, &target_id).await;
        } else if !pinned {
            self.reap_idle_tab(tab_id).await;
        }

        Ok(self.find_tab(tab_id)?.clone())
    }

    async fn promote(&mut self, name: String) -> Result<Instance, Error> {
        promote::check_promotable(&self.instance)?;
        self.instance.promote_to_name = Some(name);

        let row = self.instance.clone();
        self.app
            .store
            .run(move |c| instance::update(c, &row))
            .await?;

        Ok(self.instance.clone())
    }

    async fn profile_changed(&mut self, pinned: bool, idle: IdleOverrides) {
        self.profile_pinned = pinned;
        self.pinned = pinned || self.pool_member;
        self.spec.settings.idle = idle;

        if self.pinned && self.instance.state == InstanceState::Reaping {
            self.wake_instance().await;
        } else if !self.pinned {
            self.reap_idle_instance().await;
        }
    }

    async fn relaunched(
        &mut self,
        result: Result<Launched, Error>,
    ) -> Option<mpsc::Receiver<DriverEvent>> {
        let launched = match result {
            Ok(launched) => launched,
            Err(error) => {
                fail_launch(&self.app, &mut self.instance, &error).await;

                return None;
            }
        };

        self.app.supervisor.track(launched.pid);
        if let Some(xvfb_pid) = launched.xvfb_pid {
            self.app.supervisor.track_xvfb(self.instance.id, xvfb_pid);
        }
        self.instance.pid = Some(launched.pid);
        self.instance.started_at = Some(now_ms());
        let options = self
            .app
            .content
            .options(
                &self.app,
                self.spec.settings.block_ads,
                self.spec.settings.dismiss_cookies,
            )
            .await;
        if let Err(error) = launched.driver.set_content_defaults(options).await {
            warn!(instance_id = %self.instance.id, %error, "content defaults not applied");
        }
        self.set_driver(Some(launched.driver));

        let applied = self.transition(InstanceTrigger::Ready, None).await;
        if let Err(error) = applied {
            warn!(instance_id = %self.instance.id, %error, "restart not recorded");
        }

        if let Some(reason) = self.pending_stop.take()
            && let Err(error) = self.begin_stop(InstanceTrigger::Stop, reason).await
        {
            warn!(instance_id = %self.instance.id, %error, "stop not applied");
        }

        Some(launched.events)
    }
}
