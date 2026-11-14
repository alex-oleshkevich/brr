use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex, PoisonError, Weak},
    time::Duration,
};

use brr_core::{
    AdmissionInputs, AdmissionMode, AdmissionOutcome, Display, EndReason, Engine, Error, EventData,
    InstanceId, InstanceKind, InstanceState, Settings, TabState, settings::LimitMode,
};
use schemars::JsonSchema;
use serde::Serialize;
use tokio::{
    sync::{Mutex, oneshot, watch},
    time,
};
use tracing::{debug, info, warn};

use crate::{
    app::App,
    internal,
    services::{
        MIN_CHECK_S,
        host::{self, BYTES_PER_MB},
    },
};
use decide::{Class, Snapshot, Verdict, class, decide};

mod decide;

#[cfg(test)]
mod tests;

// A stop that reaches a launch before it has an owner; the launch task ends the row.
pub type Cancel = watch::Receiver<Option<EndReason>>;

pub const RETRY_AFTER_S: u64 = 10;
pub const RESERVATION_HOLD: Duration = Duration::from_secs(30);
pub const EWMA_ALPHA: f64 = 0.3;

pub struct AdmitRequest {
    pub instance_id: InstanceId,
    pub profile_name: String,
    pub engine: Engine,
    pub display: Display,
    pub tags: Vec<String>,
    pub priority: u8,
    pub mode: AdmissionMode,
    pub queued_at: i64,
}

#[derive(Serialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    pub instance_id: InstanceId,
    pub profile_name: String,
    pub priority: u8,
    pub queue_pos: u32,
    pub queued_at: i64,
}

// Decisions are serialized (A1): every read-decide-record step runs under this one lock, which
// is never held while a request waits in the queue or an evicted Instance stops.
pub struct Admission {
    state: Mutex<State>,
    // Tab opens in flight: the owner has not listed those Tabs yet.
    opening_tabs: Arc<StdMutex<usize>>,
}

// Held while a Tab opens; dropping it frees the slot.
pub struct TabSlot(Arc<StdMutex<usize>>);

impl Drop for TabSlot {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) -= 1;
    }
}

struct State {
    admitted: HashMap<InstanceId, Admitted>,
    reservations: HashMap<InstanceId, u64>,
    // Victims whose stop is in flight: their slot is as good as free, and nobody picks them twice.
    evicting: HashSet<InstanceId>,
    queue: Vec<Waiter>,
    estimates: Estimates,
    next_seq: u64,
    timer_running: bool,
}

struct Admitted {
    engine: Engine,
    tags: Vec<String>,
    class: Class,
}

struct Waiter {
    request: AdmitRequest,
    seq: u64,
    done: oneshot::Sender<Result<(), Error>>,
}

// Memory one new Instance of each class is expected to need, in MB.
struct Estimates {
    chromium_headless: u64,
    chromium_display: u64,
    camoufox: u64,
}

struct Host {
    free_ram_mb: u64,
    load_per_core: f64,
    // What each reapable Instance's process tree uses now: removing it frees that much, though
    // its reservation expired long ago.
    tree_mb: HashMap<InstanceId, u64>,
}

impl Estimates {
    fn get(&self, class: Class) -> u64 {
        match class {
            Class::ChromiumHeadless => self.chromium_headless,
            Class::ChromiumDisplay => self.chromium_display,
            Class::Camoufox => self.camoufox,
        }
    }

    fn slot(&mut self, class: Class) -> &mut u64 {
        match class {
            Class::ChromiumHeadless => &mut self.chromium_headless,
            Class::ChromiumDisplay => &mut self.chromium_display,
            Class::Camoufox => &mut self.camoufox,
        }
    }
}

impl Admission {
    pub fn new(settings: &Settings) -> Admission {
        let reserve = &settings.adaptive.reserve_mb;

        Admission {
            state: Mutex::new(State {
                admitted: HashMap::new(),
                reservations: HashMap::new(),
                evicting: HashSet::new(),
                queue: Vec::new(),
                estimates: Estimates {
                    chromium_headless: reserve.chromium_headless,
                    chromium_display: reserve.chromium_display,
                    camoufox: reserve.camoufox,
                },
                next_seq: 0,
                timer_running: false,
            }),
            opening_tabs: Arc::new(StdMutex::new(0)),
        }
    }

    pub async fn admit(
        &self,
        app: &Arc<App>,
        request: AdmitRequest,
        cancel: &mut Cancel,
    ) -> Result<(), Error> {
        let instance_id = request.instance_id;
        let settings = app.settings();
        let mut waiting = {
            let mut state = self.state.lock().await;
            if cancel.borrow().is_some() {
                return Err(cap_exceeded(None));
            }
            // Waiters were here first: room that appeared since the last pump is theirs.
            if !state.queue.is_empty() {
                self.pump_locked(app, &mut state).await;
            }
            let mut host = read_host()?;
            let (verdict, inputs) =
                evaluate(&state, &settings, &host, &request, &HashSet::new(), &[]);

            if let Verdict::Full { reason } = &verdict {
                debug!(%instance_id, reason, "admission is full");
            } else {
                commit(&mut state, &request);
                decided(app, instance_id, AdmissionOutcome::Admitted, inputs).await;

                return Ok(());
            }

            match request.mode {
                AdmissionMode::Reject => {
                    decided(app, instance_id, AdmissionOutcome::Rejected, inputs).await;

                    return Err(cap_exceeded(Some(RETRY_AFTER_S)));
                }
                AdmissionMode::Evict => {
                    // No candidate: the request just queues (A2a).
                    host.tree_mb = tree_memory(app).await;
                    if let Some(victim) = evict_candidate(app, &state, &settings, &host, &request) {
                        decided(app, instance_id, AdmissionOutcome::Evicted, inputs.clone()).await;
                        state.evicting.insert(victim);
                        evict(app, victim);
                    }
                }
                AdmissionMode::Queue => {}
            }

            let (done, waiting) = oneshot::channel();
            state.next_seq += 1;
            let seq = state.next_seq;
            let priority = request.priority;
            state.queue.push(Waiter { request, seq, done });
            let queue_pos = position(&state.queue, &instance_id);
            publish(
                app,
                EventData::InstanceQueued {
                    instance_id,
                    queue_pos,
                    priority,
                },
            )
            .await;
            decided(app, instance_id, AdmissionOutcome::Queued, inputs).await;
            self.ensure_timer(app, &mut state);

            waiting
        };

        let wait = Duration::from_secs(settings.admission.queue_timeout_s);
        tokio::select! {
            admitted = time::timeout(wait, &mut waiting) => match admitted {
                Ok(Ok(admitted)) => admitted,
                Ok(Err(_)) => Err(internal("admission dropped the request")),
                Err(_) => {
                    let timeout = cap_exceeded(Some(RETRY_AFTER_S));

                    self.leave_queue(&instance_id, waiting, timeout).await
                }
            },
            _ = cancelled(cancel) => {
                self.leave_queue(&instance_id, waiting, cap_exceeded(None)).await
            }
        }
    }

    // The reservation is held for RESERVATION_HOLD, then replaced by the measured memory.
    pub async fn ready(&self, app: &Arc<App>, instance_id: &InstanceId) {
        // Weak: a strong reference would keep the daemon lock for the whole hold.
        let app = Arc::downgrade(app);
        let instance_id = *instance_id;

        tokio::spawn(async move {
            time::sleep(RESERVATION_HOLD).await;
            if let Some(app) = app.upgrade() {
                app.admission.sample(&app, &instance_id).await;
            }
        });
    }

    pub async fn finished(&self, app: &Arc<App>, instance_id: &InstanceId) {
        let mut state = self.state.lock().await;
        state.reservations.remove(instance_id);
        state.evicting.remove(instance_id);

        if state.admitted.remove(instance_id).is_some() {
            self.pump_locked(app, &mut state).await;
        }
    }

    pub async fn pump(&self, app: &Arc<App>) {
        let mut state = self.state.lock().await;

        self.pump_locked(app, &mut state).await;
    }

    pub async fn queue(&self) -> Vec<QueueEntry> {
        let state = self.state.lock().await;

        ordered(&state.queue)
            .into_iter()
            .zip(1..)
            .map(|(waiter, queue_pos)| QueueEntry {
                instance_id: waiter.request.instance_id,
                profile_name: waiter.request.profile_name.clone(),
                priority: waiter.request.priority,
                queue_pos,
                queued_at: waiter.request.queued_at,
            })
            .collect()
    }

    // Opening a Tab never queues: it fails and the caller retries. The per-Instance limit is
    // the owner's to check; the total needs the slot, since the Tab is not listed until the
    // browser has opened it.
    pub fn check_tab(&self, app: &Arc<App>) -> Result<TabSlot, Error> {
        let max_total = app.settings().limits.max_tabs_total as usize;
        let open: usize = app
            .instances
            .views()
            .iter()
            // A Live browser's own Tabs are the user's, not brr's load.
            .filter(|view| view.instance.kind == InstanceKind::Managed)
            .map(|view| {
                view.tabs
                    .iter()
                    .filter(|tab| tab.state != TabState::Closing)
                    .count()
            })
            .sum();

        let mut opening = self
            .opening_tabs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if open + *opening >= max_total {
            return Err(cap_exceeded(Some(RETRY_AFTER_S)));
        }
        *opening += 1;

        Ok(TabSlot(Arc::clone(&self.opening_tabs)))
    }

    // Gives up the wait (timeout or cancel) unless the answer arrived just as it ended.
    async fn leave_queue(
        &self,
        instance_id: &InstanceId,
        mut waiting: oneshot::Receiver<Result<(), Error>>,
        error: Error,
    ) -> Result<(), Error> {
        let mut state = self.state.lock().await;
        let queued = state
            .queue
            .iter()
            .position(|waiter| waiter.request.instance_id == *instance_id);

        match queued {
            Some(pos) => {
                state.queue.remove(pos);

                Err(error)
            }
            // Admitted just as the wait ended: that answer stands.
            None => waiting
                .try_recv()
                .unwrap_or_else(|_| Err(internal("admission dropped the request"))),
        }
    }

    async fn sample(&self, app: &Arc<App>, instance_id: &InstanceId) {
        let class = {
            let state = self.state.lock().await;
            match state.admitted.get(instance_id) {
                Some(admitted) => admitted.class,
                None => return,
            }
        };

        let browsers: Vec<_> = app
            .instances
            .view(instance_id)
            .and_then(|view| view.instance.pid)
            .map(|pid| (*instance_id, pid))
            .into_iter()
            .collect();
        let measured = host::pss_by_instance(browsers)
            .await
            .get(instance_id)
            .copied();

        let mut state = self.state.lock().await;
        if !state.admitted.contains_key(instance_id) {
            return;
        }
        if let Some(bytes) = measured.filter(|bytes| *bytes > 0) {
            let estimate = state.estimates.slot(class);
            *estimate = ewma(*estimate, bytes / BYTES_PER_MB);
        }
        state.reservations.remove(instance_id);

        self.pump_locked(app, &mut state).await;
    }

    // Highest priority first, FIFO within a priority; a waiter the formula rejects does not
    // block the ones behind it.
    async fn pump_locked(&self, app: &Arc<App>, state: &mut State) {
        while !state.queue.is_empty() {
            let settings = app.settings();
            let host = match read_host() {
                Ok(host) => host,
                Err(error) => {
                    warn!(%error, "queued instances not admitted: host not readable");

                    break;
                }
            };

            let next = {
                let waiters = ordered(&state.queue);
                waiters.into_iter().find_map(|waiter| {
                    let (verdict, inputs) = evaluate(
                        state,
                        &settings,
                        &host,
                        &waiter.request,
                        &HashSet::new(),
                        &[],
                    );

                    (verdict == Verdict::Admit).then_some((waiter.request.instance_id, inputs))
                })
            };
            let Some((instance_id, inputs)) = next else {
                break;
            };

            let Some(pos) = state
                .queue
                .iter()
                .position(|waiter| waiter.request.instance_id == instance_id)
            else {
                break;
            };
            let waiter = state.queue.remove(pos);
            commit(state, &waiter.request);
            decided(app, instance_id, AdmissionOutcome::Admitted, inputs).await;

            if waiter.done.send(Ok(())).is_err() {
                // The caller is gone: its slot must not stay taken.
                state.admitted.remove(&instance_id);
                state.reservations.remove(&instance_id);
            }
        }

        self.ensure_timer(app, state);
    }

    // The re-check runs only while something waits and the formula can change by itself.
    fn ensure_timer(&self, app: &Arc<App>, state: &mut State) {
        if state.timer_running
            || state.queue.is_empty()
            || app.settings().limits.mode != LimitMode::Adaptive
        {
            return;
        }

        state.timer_running = true;
        tokio::spawn(timer_loop(Arc::downgrade(app)));
    }
}

async fn timer_loop(app: Weak<App>) {
    loop {
        let Some(strong) = app.upgrade() else {
            return;
        };
        let check_s = strong.settings().adaptive.pressure_check_s.max(MIN_CHECK_S);
        drop(strong);
        time::sleep(Duration::from_secs(check_s)).await;

        let Some(strong) = app.upgrade() else {
            return;
        };
        let admission = &strong.admission;
        let mut state = admission.state.lock().await;
        admission.pump_locked(&strong, &mut state).await;
        if state.queue.is_empty() || strong.settings().limits.mode != LimitMode::Adaptive {
            state.timer_running = false;

            return;
        }
    }
}

pub async fn cancelled(cancel: &mut Cancel) -> EndReason {
    let reason = cancel.wait_for(Option::is_some).await.map(|reason| *reason);

    match reason {
        Ok(reason) => reason.unwrap_or(EndReason::Cancel),
        // The launch task owns the sender for as long as it runs.
        Err(_) => std::future::pending().await,
    }
}

pub fn ewma(estimate_mb: u64, sample_mb: u64) -> u64 {
    (EWMA_ALPHA * sample_mb as f64 + (1.0 - EWMA_ALPHA) * estimate_mb as f64).round() as u64
}

fn cap_exceeded(retry_after: Option<u64>) -> Error {
    Error::CapExceeded {
        retry_after,
        queue_pos: None,
    }
}

fn read_host() -> Result<Host, Error> {
    Ok(Host {
        free_ram_mb: host::memory()?.available_mb,
        load_per_core: host::load_per_core()?,
        tree_mb: HashMap::new(),
    })
}

async fn tree_memory(app: &Arc<App>) -> HashMap<InstanceId, u64> {
    let browsers: Vec<_> = app
        .instances
        .views()
        .into_iter()
        .filter_map(|view| Some((view.instance.id, view.instance.pid?)))
        .collect();
    let bytes = host::pss_by_instance(browsers).await;

    bytes
        .into_iter()
        .map(|(instance_id, bytes)| (instance_id, bytes / BYTES_PER_MB))
        .collect()
}

fn ordered(queue: &[Waiter]) -> Vec<&Waiter> {
    let mut waiters: Vec<&Waiter> = queue.iter().collect();
    waiters.sort_by_key(|waiter| (Reverse(waiter.request.priority), waiter.seq));

    waiters
}

fn position(queue: &[Waiter], instance_id: &InstanceId) -> u32 {
    let pos = ordered(queue)
        .iter()
        .position(|waiter| waiter.request.instance_id == *instance_id)
        .map_or(0, |pos| pos + 1);

    u32::try_from(pos).unwrap_or(u32::MAX)
}

fn evaluate(
    state: &State,
    settings: &Settings,
    host: &Host,
    request: &AdmitRequest,
    removed: &HashSet<InstanceId>,
    ahead: &[&AdmitRequest],
) -> (Verdict, AdmissionInputs) {
    let estimate_mb = state.estimates.get(class(request.engine, request.display));
    // Requests queued ahead take their room first, so they count as admitted.
    let ahead_mb: u64 = ahead
        .iter()
        .map(|waiting| state.estimates.get(class(waiting.engine, waiting.display)))
        .sum();
    let reserved_mb: u64 = state
        .reservations
        .iter()
        .filter(|(id, _)| !removed.contains(id))
        .map(|(_, mb)| mb)
        .sum::<u64>()
        + ahead_mb;
    // A removed Instance without a reservation has its memory counted as used in MemAvailable.
    let freed_mb: u64 = removed
        .iter()
        .filter(|id| !state.reservations.contains_key(id))
        .filter_map(|id| {
            let admitted = state.admitted.get(id)?;

            Some(
                host.tree_mb
                    .get(id)
                    .copied()
                    .unwrap_or_else(|| state.estimates.get(admitted.class)),
            )
        })
        .sum();
    let free_ram_mb = host.free_ram_mb + freed_mb;
    let admitted: Vec<(Engine, Vec<String>)> = state
        .admitted
        .iter()
        .filter(|(id, _)| !removed.contains(id))
        .map(|(_, admitted)| (admitted.engine, admitted.tags.clone()))
        .chain(
            ahead
                .iter()
                .map(|waiting| (waiting.engine, waiting.tags.clone())),
        )
        .collect();

    let verdict = decide(
        &Snapshot {
            settings,
            admitted: &admitted,
            free_ram_mb,
            reserved_mb,
            load_per_core: host.load_per_core,
            estimate_mb,
        },
        request.engine,
        &request.tags,
    );

    let on_engine = |engine| admitted.iter().filter(|(e, _)| *e == engine).count();
    let mut counts = BTreeMap::from([
        ("total".to_owned(), count(admitted.len())),
        ("chromium".to_owned(), count(on_engine(Engine::Chromium))),
        ("camoufox".to_owned(), count(on_engine(Engine::Camoufox))),
    ]);
    for tag in &request.tags {
        let with_tag = admitted
            .iter()
            .filter(|(_, tags)| tags.contains(tag))
            .count();
        counts.insert(format!("tag:{tag}"), count(with_tag));
    }

    let inputs = AdmissionInputs {
        free_ram_mb: host.free_ram_mb,
        // The candidate's own estimate is included, so the event alone proves the formula.
        reserved_mb: reserved_mb + estimate_mb,
        load_per_core: host.load_per_core,
        counts,
    };

    (verdict, inputs)
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn commit(state: &mut State, request: &AdmitRequest) {
    let class = class(request.engine, request.display);

    state
        .reservations
        .insert(request.instance_id, state.estimates.get(class));
    state.admitted.insert(
        request.instance_id,
        Admitted {
            engine: request.engine,
            tags: request.tags.clone(),
            class,
        },
    );
}

// The least recently active idle or reaping Instance nobody pinned, and only one whose removal
// (with the evictions already under way) lets the request in: killing an idle camoufox for a
// chromium request that waits on the chromium ceiling would free nothing.
fn evict_candidate(
    app: &Arc<App>,
    state: &State,
    settings: &Settings,
    host: &Host,
    request: &AdmitRequest,
) -> Option<InstanceId> {
    let ahead: Vec<&AdmitRequest> = state.queue.iter().map(|waiter| &waiter.request).collect();
    let admits = |removed: &HashSet<InstanceId>| {
        evaluate(state, settings, host, request, removed, &ahead).0 == Verdict::Admit
    };
    if admits(&state.evicting) {
        return None;
    }

    let mut views = app.instances.views();
    views.sort_by_key(|view| view.instance.last_activity_at);

    views
        .into_iter()
        .filter(|view| {
            matches!(
                view.instance.state,
                InstanceState::Idle | InstanceState::Reaping
            ) && view.instance.kind == InstanceKind::Managed
                && !view.pinned
                && !state.evicting.contains(&view.instance.id)
        })
        .map(|view| view.instance.id)
        .find(|id| {
            let mut removed = state.evicting.clone();
            removed.insert(*id);

            admits(&removed)
        })
}

fn evict(app: &Arc<App>, instance_id: InstanceId) {
    let app = Arc::clone(app);

    tokio::spawn(async move {
        let stopped = app
            .instances
            .stop(&app, &instance_id, EndReason::Evicted)
            .await;
        // Refused (activity or a pin won the race) or done: either way the victim is no longer
        // being evicted.
        app.admission
            .state
            .lock()
            .await
            .evicting
            .remove(&instance_id);

        match stopped {
            Ok(_) | Err(Error::InvalidTransition { .. }) => {}
            Err(error) => warn!(%instance_id, %error, "evicted instance not stopped"),
        }
    });
}

async fn decided(
    app: &Arc<App>,
    instance_id: InstanceId,
    outcome: AdmissionOutcome,
    inputs: AdmissionInputs,
) {
    info!(
        %instance_id,
        ?outcome,
        free_ram_mb = inputs.free_ram_mb,
        reserved_mb = inputs.reserved_mb,
        load_per_core = inputs.load_per_core,
        counts = ?inputs.counts,
        "admission decided"
    );
    publish(
        app,
        EventData::AdmissionDecided {
            instance_id,
            outcome,
            inputs,
        },
    )
    .await;
}

// The decision stands even when its record cannot be written.
async fn publish(app: &Arc<App>, data: EventData) {
    if let Err(error) = app.events.publish(data).await {
        warn!(%error, "admission event not published");
    }
}
