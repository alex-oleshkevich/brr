use std::{
    collections::HashSet,
    fmt::Write,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use brr_captcha::{
    apply::apply_js,
    detect::{DETECT_JS, Detected, parse_detected},
    provider::{Challenge, Provider, ProxyTask, Solution, Solved, TwoCaptcha},
};
use brr_core::{
    CaptchaPolicy, CaptchaResult, CaptchaSolve, CaptchaSolveId, CaptchaType, ConflictReason, Error,
    EventData, HumanReason, InstanceId, MICROS_PER_USD, ProfileId, Secret, TabId, TabState,
    TabTrigger, UsdMicros, capture::ImageFormat, clock::local_day_start, now_ms, site_origin,
};
use brr_engine::page::{Clip, ScreenshotOptions, Target};
use brr_netproxy::Upstream;
use brr_store::{captcha_solve, instance, profile, secret};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::json;
use tokio::sync::{Mutex as AsyncMutex, broadcast::error::RecvError};
use tracing::{info, warn};

use crate::{app::App, internal, ops::TabCall, services::proxies::instance_upstream};

#[cfg(test)]
mod tests;

pub const API_KEY_SECRET: &str = "captcha.api_key";
pub use brr_captcha::provider::PROVIDER_NAME;

const STATE_CHANGED: &str = "tab.state_changed";
const SOLVE_TIMEOUT: Duration = Duration::from_secs(180);
const APPLY_TIMEOUT: Duration = Duration::from_secs(10);
// What each in-flight solve counts against the daily USD cap until its real cost is recorded:
// above 2captcha's dearest task, which costs about 0.003 USD.
const RESERVED_SOLVE_MICROS: u64 = 5_000;

#[derive(Serialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SolvedBy {
    Provider,
    Human,
}

// Per-daemon state; the counters restart with the daemon, the solve log is the durable record.
#[derive(Default)]
pub struct Captcha {
    in_flight: Mutex<HashSet<TabId>>,
    exhausted_day: Mutex<Option<i64>>,
    // Held from the budget check to the reservation, so concurrent flows cannot all pass it.
    budget_gate: AsyncMutex<()>,
    pending: AtomicU32,
    // Solves paid for whose row could not be stored: the table cannot show them, so the budget
    // counts them from memory until the daemon restarts.
    unrecorded_solves: AtomicU32,
    unrecorded_micros: AtomicU64,
    spent_micros: AtomicU64,
    solved: AtomicU64,
    failed: AtomicU64,
}

struct FlowGuard {
    app: Arc<App>,
    tab_id: TabId,
}

impl Drop for FlowGuard {
    fn drop(&mut self) {
        self.app
            .captcha
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.tab_id);
    }
}

// A provider task that has been paid for or is about to be; counts against the daily solves
// until its row is in the table.
struct Reservation(Arc<App>);

impl Drop for Reservation {
    fn drop(&mut self) {
        self.0.captcha.pending.fetch_sub(1, Ordering::Relaxed);
    }
}

struct Flow<'a> {
    app: &'a Arc<App>,
    call: &'a TabCall,
    detected: Detected,
    site: String,
    failure_published: AtomicBool,
}

impl Captcha {
    // Held weakly: a strong reference would keep the daemon lock after the last owner drops the App.
    pub fn start(app: &Arc<App>) {
        let weak = Arc::downgrade(app);
        let mut events = app.events.subscribe();

        tokio::spawn(async move {
            loop {
                let published = match events.recv().await {
                    Ok(published) => published,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                };
                if published.event.kind != STATE_CHANGED {
                    continue;
                }
                let Ok(EventData::TabStateChanged {
                    tab_id,
                    instance_id,
                    to: TabState::Ready,
                    trigger: TabTrigger::Loaded,
                    ..
                }) = serde_json::from_value(
                    json!({"kind": published.event.kind, "data": published.event.data}),
                )
                else {
                    continue;
                };
                let Some(app) = weak.upgrade() else {
                    return;
                };

                // Never awaited here: a slow page would make this subscriber lag.
                tokio::spawn(async move {
                    if let Err(error) = on_load(&app, instance_id, tab_id).await {
                        warn!(%tab_id, %error, "captcha flow failed");
                    }
                });
            }
        });
    }

    pub fn render_metrics(&self, out: &mut String) {
        let spent = self.spent_micros.load(Ordering::Relaxed);
        let _ = writeln!(
            out,
            "# HELP brr_captcha_spend_usd_total Captcha provider spend since the daemon started"
        );
        let _ = writeln!(out, "# TYPE brr_captcha_spend_usd_total counter");
        let _ = writeln!(
            out,
            "brr_captcha_spend_usd_total {}.{:06}",
            spent / MICROS_PER_USD,
            spent % MICROS_PER_USD
        );

        let _ = writeln!(
            out,
            "# HELP brr_captcha_solves_total Captcha provider solves since the daemon started"
        );
        let _ = writeln!(out, "# TYPE brr_captcha_solves_total counter");
        for (result, count) in [("solved", &self.solved), ("failed", &self.failed)] {
            let _ = writeln!(
                out,
                "brr_captcha_solves_total{{result=\"{result}\"}} {}",
                count.load(Ordering::Relaxed)
            );
        }
    }
}

// Detects on the Tab now and settles what it finds: the provider first, then a person.
pub async fn solve_now(
    app: &Arc<App>,
    call: &TabCall,
    human_fallback: bool,
) -> Result<Option<(Detected, SolvedBy)>, Error> {
    let Some(flow) = Flow::detect(app, call).await? else {
        return Ok(None);
    };
    let Some(_guard) = begin(app, call.tab_id) else {
        return Err(Error::Conflict {
            reason: ConflictReason::CaptchaInProgress,
            detail: None,
        });
    };
    let key = match policy_of(app, call.profile_id).await? {
        CaptchaPolicy::OnDemand | CaptchaPolicy::Auto => Some(api_key(app).await?),
        _ => None,
    };
    let solved_by = flow.resolve(key.as_ref(), human_fallback).await?;

    Ok(Some((flow.detected, solved_by)))
}

// At most one flow per Tab once a captcha is found: a Tab already in one is skipped.
fn begin(app: &Arc<App>, tab_id: TabId) -> Option<FlowGuard> {
    let fresh = app
        .captcha
        .in_flight
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(tab_id);

    fresh.then(|| FlowGuard {
        app: Arc::clone(app),
        tab_id,
    })
}

async fn on_load(app: &Arc<App>, instance_id: InstanceId, tab_id: TabId) -> Result<(), Error> {
    let profile_id = app
        .store
        .run(move |c| instance::get(c, &instance_id))
        .await?
        .profile_id;
    let policy = policy_of(app, profile_id).await?;
    if policy == CaptchaPolicy::Off {
        return Ok(());
    }

    let call = TabCall {
        instance_id,
        tab_id,
        profile_id,
        page: app.instances.page(&instance_id, &tab_id).await?,
    };
    let Some(flow) = Flow::detect(app, &call).await? else {
        return Ok(());
    };
    let Some(_guard) = begin(app, tab_id) else {
        return Ok(());
    };
    app.events
        .publish(EventData::CaptchaDetected {
            tab_id,
            instance_id,
            site: flow.site.clone(),
            captcha_type: type_name(flow.detected.captcha_type),
        })
        .await?;

    let settled = match policy {
        CaptchaPolicy::Auto if flow.detected.captcha_type != CaptchaType::RecaptchaV3 => {
            let key = api_key(app).await?;

            flow.resolve(Some(&key), true).await.map(|_| ())
        }
        CaptchaPolicy::Human => flow.ask_human().await,
        _ => return Ok(()),
    };

    // Nobody called for this flow, so the event is the only place its failure shows; a failed
    // provider attempt already published it.
    match settled {
        Err(error @ Error::CaptchaFailed) if !flow.failure_published.load(Ordering::Relaxed) => {
            flow.publish_failed(&error).await
        }
        other => other,
    }
}

// Without a key every Profile is off, whatever it says (K1). Only the row is looked up: the key
// is decrypted by whoever needs it, once a captcha is found.
async fn policy_of(app: &Arc<App>, profile_id: Option<ProfileId>) -> Result<CaptchaPolicy, Error> {
    let key_set = app
        .store
        .run(|c| secret::get(c, API_KEY_SECRET))
        .await?
        .is_some();
    if !key_set {
        return Ok(CaptchaPolicy::Off);
    }

    let own = match profile_id {
        Some(profile_id) => {
            app.store
                .run(move |c| profile::get(c, &profile_id))
                .await?
                .settings
                .captcha
        }
        None => None,
    };

    Ok(own.unwrap_or(app.settings().captcha.default_policy))
}

async fn api_key(app: &Arc<App>) -> Result<Secret, Error> {
    app.secrets
        .get(&app.store, API_KEY_SECRET)
        .await?
        .ok_or(Error::CaptchaFailed)
}

pub fn provider(app: &App, key: &Secret) -> Provider {
    Provider::TwoCaptcha(TwoCaptcha::new(
        app.http.clone(),
        app.settings().captcha.provider_url.clone(),
        Secret::new(key.expose()),
    ))
}

pub async fn spent_today(app: &App) -> Result<(u64, u32), Error> {
    let day_start = local_day_start(now_ms());

    app.store
        .run(move |c| captcha_solve::spent_since(c, day_start))
        .await
}

// The Profile's own upstream, never the local forwarder: the provider cannot reach 127.0.0.1.
fn proxy_task(upstream: Option<&Upstream>) -> Option<ProxyTask> {
    upstream.map(|upstream| ProxyTask {
        scheme: upstream.scheme,
        host: upstream.host.clone(),
        port: upstream.port,
        username: upstream.username.clone(),
        password: upstream
            .password
            .as_ref()
            .map(|password| Secret::new(password.expose())),
    })
}

fn type_name(captcha_type: CaptchaType) -> String {
    match serde_json::to_value(captcha_type) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

impl<'a> Flow<'a> {
    async fn detect(app: &'a Arc<App>, call: &'a TabCall) -> Result<Option<Flow<'a>>, Error> {
        let found = call.page.evaluate_utility(DETECT_JS).await?;
        let Some(detected) = parse_detected(found)? else {
            return Ok(None);
        };
        let url = call.page.url().await?;

        Ok(Some(Flow {
            app,
            call,
            detected,
            site: site_origin(&url).unwrap_or(url),
            failure_published: AtomicBool::new(false),
        }))
    }

    async fn resolve(&self, key: Option<&Secret>, human_fallback: bool) -> Result<SolvedBy, Error> {
        match key {
            Some(key) => match self.solve_and_apply(key).await {
                Ok(()) => return Ok(SolvedBy::Provider),
                Err(error) if !human_fallback => return Err(error),
                Err(_) => {}
            },
            None if !human_fallback => return Err(Error::CaptchaFailed),
            None => {}
        }

        self.ask_human().await?;

        Ok(SolvedBy::Human)
    }

    async fn ask_human(&self) -> Result<(), Error> {
        let wait = Duration::from_secs(self.app.settings().captcha.human_wait_s);
        let waited = self
            .app
            .instances
            .await_human(
                &self.call.instance_id,
                &self.call.tab_id,
                HumanReason::Captcha,
                wait,
            )
            .await;

        match waited {
            Ok(_) => Ok(()),
            Err(Error::Timeout { .. }) => Err(Error::CaptchaFailed),
            Err(error) => Err(error),
        }
    }

    async fn solve_and_apply(&self, key: &Secret) -> Result<(), Error> {
        let detected = &self.detected;
        // Nothing is paid for an answer that cannot be applied.
        if detected.captcha_type == CaptchaType::Image
            && (detected.image_selector.is_none() || detected.input_selector.is_none())
        {
            info!("image captcha without a selector");

            return Err(Error::CaptchaFailed);
        }
        let reservation = self.reserve().await?;

        let image_png = if detected.captcha_type == CaptchaType::Image {
            let selector = detected
                .image_selector
                .clone()
                .ok_or(Error::CaptchaFailed)?;
            let options = ScreenshotOptions {
                clip: Clip::Element(Target::Selector(selector)),
                format: ImageFormat::Png,
                quality: None,
                transparent: false,
            };

            Some(self.call.page.screenshot(&options).await?)
        } else {
            None
        };
        let upstream = instance_upstream(self.app, &self.call.instance_id);
        let challenge = Challenge {
            detected: detected.clone(),
            page_url: self.call.page.url().await?,
            image_png,
            proxy: proxy_task(upstream.as_deref()),
        };

        let provider = provider(self.app, key);
        let row = self.row(&provider);
        // Its own task: a caller that gives up must not lose track of money already spent.
        let paid = tokio::spawn(pay(
            Arc::clone(self.app),
            reservation,
            provider,
            challenge,
            row,
        ))
        .await
        .map_err(internal)?;
        let (outcome, duration_ms) = paid?;
        let solved = match outcome {
            Ok(solved) => solved,
            Err(error) => {
                self.publish_failed(&error).await?;

                return Err(error);
            }
        };

        if let Err(error) = self.apply(&solved.solution).await {
            info!(%error, "applying the captcha solution failed");
            self.publish_failed(&error).await?;

            return Err(Error::CaptchaFailed);
        }

        self.app
            .events
            .publish(EventData::CaptchaSolved {
                tab_id: self.call.tab_id,
                instance_id: self.call.instance_id,
                site: self.site.clone(),
                captcha_type: type_name(detected.captcha_type),
                cost_usd: UsdMicros(solved.cost_usd_micros),
                duration_ms,
            })
            .await?;

        Ok(())
    }

    async fn reserve(&self) -> Result<Reservation, Error> {
        let captcha = &self.app.captcha;
        let limits = self.app.settings().captcha.clone();
        let _gate = captcha.budget_gate.lock().await;
        let (stored_spent, stored_solves) = spent_today(self.app).await?;
        let spent = stored_spent.saturating_add(captcha.unrecorded_micros.load(Ordering::Relaxed));
        let solves =
            stored_solves.saturating_add(captcha.unrecorded_solves.load(Ordering::Relaxed));
        let pending = captcha.pending.load(Ordering::Relaxed);
        let reserved = u64::from(pending) * RESERVED_SOLVE_MICROS;
        if spent.saturating_add(reserved) < limits.daily_usd.0
            && solves + pending < limits.daily_solves
        {
            captcha.pending.fetch_add(1, Ordering::Relaxed);

            return Ok(Reservation(Arc::clone(self.app)));
        }

        let today = local_day_start(now_ms());
        let first_today = {
            let mut seen = self
                .app
                .captcha
                .exhausted_day
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let first = *seen != Some(today);
            *seen = Some(today);

            first
        };
        if first_today {
            self.app
                .events
                .publish(EventData::CaptchaBudgetExhausted {
                    spent_usd: UsdMicros(spent),
                    solves,
                })
                .await?;
        }

        Err(Error::CaptchaBudgetExceeded)
    }

    // The log row of a solve that succeeded; `pay` fills in what the provider answered.
    fn row(&self, provider: &Provider) -> CaptchaSolve {
        CaptchaSolve {
            id: CaptchaSolveId::new(),
            ts: now_ms(),
            profile_id: self.call.profile_id,
            tab_id: Some(self.call.tab_id),
            site: self.site.clone(),
            captcha_type: self.detected.captcha_type,
            provider: provider.name().to_string(),
            cost_usd_micros: 0,
            duration_ms: 0,
            result: CaptchaResult::Solved,
            error: None,
        }
    }

    async fn apply(&self, solution: &Solution) -> Result<(), Error> {
        let page = &self.call.page;

        if let Solution::Text(text) = solution {
            let selector = self
                .detected
                .input_selector
                .clone()
                .ok_or(Error::CaptchaFailed)?;

            return page
                .fill(&Target::Selector(selector), text.expose(), APPLY_TIMEOUT)
                .await;
        }

        let script = apply_js(&self.detected, solution).ok_or(Error::CaptchaFailed)?;
        // brr's own script: it must also work on a page where a login typed a secret.
        page.evaluate_own(&script, APPLY_TIMEOUT).await?;

        Ok(())
    }

    async fn publish_failed(&self, error: &Error) -> Result<(), Error> {
        self.failure_published.store(true, Ordering::Relaxed);
        self.app
            .events
            .publish(EventData::CaptchaFailed {
                tab_id: self.call.tab_id,
                instance_id: self.call.instance_id,
                site: self.site.clone(),
                captcha_type: type_name(self.detected.captcha_type),
                error: error.to_string(),
            })
            .await?;

        Ok(())
    }
}

// Solves, then records, counts and releases the reservation, whatever became of the caller.
// The provider's spend counts when it answers; applying the answer is the flow's business.
async fn pay(
    app: Arc<App>,
    _reservation: Reservation,
    provider: Provider,
    challenge: Challenge,
    mut row: CaptchaSolve,
) -> Result<(Result<Solved, Error>, u64), Error> {
    let started = Instant::now();
    let outcome = provider.solve(&challenge, SOLVE_TIMEOUT).await;
    row.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    row.ts = now_ms();

    match &outcome {
        Ok(solved) => {
            row.cost_usd_micros = solved.cost_usd_micros;
            app.captcha
                .spent_micros
                .fetch_add(solved.cost_usd_micros, Ordering::Relaxed);
            app.captcha.solved.fetch_add(1, Ordering::Relaxed);
        }
        Err(error) => {
            row.result = CaptchaResult::Failed;
            row.error = Some(error.to_string());
            app.captcha.failed.fetch_add(1, Ordering::Relaxed);
        }
    }
    let duration_ms = row.duration_ms;
    let cost_usd_micros = row.cost_usd_micros;
    let stored = app.store.run(move |c| captcha_solve::insert(c, &row)).await;
    if stored.is_err() {
        app.captcha
            .unrecorded_solves
            .fetch_add(1, Ordering::Relaxed);
        app.captcha
            .unrecorded_micros
            .fetch_add(cost_usd_micros, Ordering::Relaxed);
    }
    stored?;

    Ok((outcome, duration_ms))
}
