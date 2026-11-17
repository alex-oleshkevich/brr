use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, PoisonError, Weak},
    time::Duration,
};

use brr_core::{
    ConflictReason, CredentialId, Error, EventData, InstanceId, InstanceState, LoginCheck,
    LoginCheckId, LoginStatus, ProfileId, TabCloseReason, TabId, WaitUntil, now_ms,
};
use brr_engine::Navigated;
use brr_engine::page::{Page, WaitCondition};
use brr_store::{credential, login_check};
use futures::{FutureExt, future::BoxFuture};
use serde_json::json;
use tokio::{sync::broadcast::error::RecvError, task::AbortHandle, time::MissedTickBehavior};
use tracing::{info, warn};

use crate::{app::App, internal, ops::TabCall, services::login::login};

#[cfg(test)]
mod tests;

const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const EXPECT_TIMEOUT: Duration = Duration::from_secs(5);
const RELOGIN_TIMEOUT: Duration = Duration::from_secs(60);
const RELOGIN_FAILURE_LIMIT: u32 = 3;
const SECONDS_PER_MINUTE: u64 = 60;
const STATE_CHANGED: &str = "instance.state_changed";
const LAUNCH_HELP: &str = "launch the profile first";

// Runs every check of a Profile on one of its Instances; false once the daemon is gone, which
// ends the interval that called it.
pub type RunProfile = Arc<dyn Fn(ProfileId, InstanceId) -> BoxFuture<'static, bool> + Send + Sync>;

// What auto re-login depends on: when any of it changes, the failures stop counting.
#[derive(PartialEq)]
struct BackoffKey {
    url: String,
    expect_selector: Option<String>,
    expect_text: Option<String>,
    credential_id: CredentialId,
    credential_updated_at: i64,
}

struct Backoff {
    profile_id: ProfileId,
    key: BackoffKey,
    failures: u32,
}

// One interval task per running Instance, keyed by it so any end state aborts it.
#[derive(Default)]
pub struct LoginChecks {
    timers: Mutex<HashMap<InstanceId, AbortHandle>>,
    // Tabs where an auto re-login runs: its own check_site call must not record or publish, the
    // run does that once.
    relogging: Mutex<HashSet<TabId>>,
    // In memory, like the timers: a restart gives a suspended check its attempts back.
    backoff: Mutex<HashMap<LoginCheckId, Backoff>>,
}

struct Relogging {
    app: Arc<App>,
    tab_id: TabId,
}

impl Relogging {
    fn start(app: &Arc<App>, tab_id: TabId) -> Relogging {
        app.login_checks
            .relogging
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(tab_id);

        Relogging {
            app: Arc::clone(app),
            tab_id,
        }
    }
}

impl Drop for Relogging {
    fn drop(&mut self) {
        self.app
            .login_checks
            .relogging
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.tab_id);
    }
}

fn ends_checking(state: InstanceState) -> bool {
    state == InstanceState::Stopping || state.is_final()
}

impl LoginChecks {
    // Held weakly: a strong reference would keep the daemon lock after the last owner drops the App.
    pub fn spawn(app: &Arc<App>) {
        let weak = Arc::downgrade(app);
        let mut events = app.events.subscribe();
        let run = run_profile(weak.clone());

        tokio::spawn(async move {
            loop {
                let published = match events.recv().await {
                    Ok(published) => published,
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                };
                let event = &published.event;
                if event.kind != STATE_CHANGED {
                    continue;
                }
                let Ok(data) = serde_json::from_value::<EventData>(
                    json!({ "kind": event.kind, "data": event.data }),
                ) else {
                    continue;
                };
                let Some(app) = weak.upgrade() else {
                    return;
                };
                let minutes = u64::from(app.settings().login_check.interval_min).max(1);
                let interval = Duration::from_secs(minutes * SECONDS_PER_MINUTE);

                app.login_checks.on_event(&run, &data, interval);
            }
        });
    }

    // Never awaits a check: they run in their own tasks, so the subscription never lags.
    // False while the check sits at the failure limit and nothing it depends on has changed.
    fn relogin_allowed(&self, check_id: &LoginCheckId, key: &BackoffKey) -> bool {
        let mut backoff = self.backoff.lock().unwrap_or_else(PoisonError::into_inner);
        match backoff.get(check_id) {
            Some(state) if state.key != *key => {
                backoff.remove(check_id);

                true
            }
            Some(state) => state.failures < RELOGIN_FAILURE_LIMIT,
            None => true,
        }
    }

    // The failure count once this outcome reaches the limit; None otherwise.
    fn relogin_outcome(
        &self,
        check_id: &LoginCheckId,
        profile_id: &ProfileId,
        key: BackoffKey,
        ok: bool,
    ) -> Option<u32> {
        let mut backoff = self.backoff.lock().unwrap_or_else(PoisonError::into_inner);
        if ok {
            backoff.remove(check_id);

            return None;
        }
        if backoff.get(check_id).is_some_and(|state| state.key != key) {
            backoff.remove(check_id);
        }
        let state = backoff.entry(*check_id).or_insert(Backoff {
            profile_id: *profile_id,
            key,
            failures: 0,
        });
        state.failures += 1;

        (state.failures == RELOGIN_FAILURE_LIMIT).then_some(state.failures)
    }

    // A deleted check never runs again, so nothing else would drop its entry.
    fn forget_deleted(&self, profile_id: &ProfileId, live: &HashSet<LoginCheckId>) {
        self.backoff
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|check_id, state| state.profile_id != *profile_id || live.contains(check_id));
    }

    pub fn on_event(&self, run: &RunProfile, data: &EventData, interval: Duration) {
        let EventData::InstanceStateChanged {
            instance_id,
            profile_id: Some(profile_id),
            from,
            to,
            ..
        } = data
        else {
            return;
        };
        let mut timers = self.timers.lock().unwrap_or_else(PoisonError::into_inner);

        if ends_checking(*to) {
            if let Some(timer) = timers.remove(instance_id) {
                timer.abort();
            }
        } else if *to == InstanceState::Running && *from == Some(InstanceState::Starting) {
            let (run, profile_id, instance_id) = (run.clone(), *profile_id, *instance_id);
            let timer = tokio::spawn(async move {
                if !run(profile_id, instance_id).await {
                    return;
                }

                let mut ticks = tokio::time::interval(interval);
                // A run that outlasts the interval waits a full one more, never a burst.
                ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
                // The first tick is now: the run above already was the check at launch.
                ticks.tick().await;
                loop {
                    ticks.tick().await;
                    if !run(profile_id, instance_id).await {
                        return;
                    }
                }
            });
            if let Some(old) = timers.insert(instance_id, timer.abort_handle()) {
                old.abort();
            }
        }
    }

    pub async fn run(
        &self,
        app: &Arc<App>,
        login_check_id: &LoginCheckId,
    ) -> Result<LoginCheck, Error> {
        let id = *login_check_id;
        let check = app.store.run(move |c| login_check::get(c, &id)).await?;
        let instance_id = running_instance(app, &check.profile_id)?;

        run_check(app, &check, &instance_id).await?;

        app.store.run(move |c| login_check::get(c, &id)).await
    }

    // The Profile's check for `site`, evaluated in the login's own Tab after loading its URL.
    // None when the Profile has none for the site.
    pub async fn check_site(
        &self,
        app: &Arc<App>,
        tab: &TabCall,
        site: &str,
    ) -> Result<Option<bool>, Error> {
        let Some(profile_id) = tab.profile_id else {
            return Ok(None);
        };
        let checks = app
            .store
            .run(move |c| login_check::for_profile(c, &profile_id))
            .await?;
        let Some(check) = checks.into_iter().find(|check| check.site == site) else {
            return Ok(None);
        };

        let ok = evaluate(app, &tab.instance_id, &tab.tab_id, &tab.page, &check).await?;
        let quiet = self
            .relogging
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&tab.tab_id);
        if !quiet {
            record(app, &check, ok).await?;
        }

        Ok(Some(ok))
    }
}

fn run_profile(app: Weak<App>) -> RunProfile {
    Arc::new(move |profile_id, instance_id| {
        let app = app.upgrade();

        async move {
            let Some(app) = app else {
                return false;
            };
            // A stop event the subscriber missed must not leave the interval running.
            let alive = app.instances.view(&instance_id).is_some_and(|view| {
                matches!(
                    view.instance.state,
                    InstanceState::Running | InstanceState::Idle | InstanceState::Reaping
                )
            });
            if !alive {
                return false;
            }
            let checks = app
                .store
                .run(move |c| login_check::for_profile(c, &profile_id))
                .await;
            // Without the list nothing is known to be deleted: the backoff stays as it is.
            let Ok(checks) = checks else {
                warn!(%profile_id, "login checks not loaded");

                return true;
            };
            let live = checks.iter().map(|check| check.id).collect();
            app.login_checks.forget_deleted(&profile_id, &live);
            for check in checks {
                if let Err(error) = run_check(&app, &check, &instance_id).await {
                    warn!(%error, %profile_id, login_check_id = %check.id, "login check did not run");
                }
            }

            true
        }
        .boxed()
    })
}

fn running_instance(app: &Arc<App>, profile_id: &ProfileId) -> Result<InstanceId, Error> {
    app.instances
        .views()
        .into_iter()
        .find(|view| {
            view.instance.profile_id == Some(*profile_id)
                && matches!(
                    view.instance.state,
                    InstanceState::Running | InstanceState::Idle
                )
        })
        .map(|view| view.instance.id)
        .ok_or_else(|| Error::Conflict {
            reason: ConflictReason::ProfileNotRunning,
            detail: Some(LAUNCH_HELP.to_owned()),
        })
}

// Loads the check's URL in the Tab and looks for what it expects. A page that does not load is a
// failed check, not an error.
async fn evaluate(
    app: &Arc<App>,
    instance_id: &InstanceId,
    tab_id: &TabId,
    page: &Page,
    check: &LoginCheck,
) -> Result<bool, Error> {
    let navigated = app
        .instances
        .navigate(
            instance_id,
            tab_id,
            check.url.clone(),
            WaitUntil::Load,
            LOAD_TIMEOUT,
        )
        .await;
    // A file instead of a page shows nothing that proves a login.
    match navigated {
        Ok((_, Navigated::Page)) => {}
        Ok((_, Navigated::Download))
        | Err(Error::NavigationFailed { .. } | Error::Timeout { .. }) => return Ok(false),
        Err(error) => return Err(error),
    }

    let expected = match (&check.expect_selector, &check.expect_text) {
        (Some(selector), _) => WaitCondition::Selector(selector.clone()),
        (None, Some(text)) => WaitCondition::Text(text.clone()),
        (None, None) => return Ok(false),
    };

    let waited = page.wait_for(&expected, EXPECT_TIMEOUT).await;
    Ok(waited.is_ok())
}

async fn record(app: &Arc<App>, check: &LoginCheck, ok: bool) -> Result<(), Error> {
    let status = if ok {
        LoginStatus::Ok
    } else {
        LoginStatus::Failed
    };
    let id = check.id;
    app.store
        .run(move |c| login_check::set_status(c, &id, status, now_ms()))
        .await?;
    app.events
        .publish(EventData::LoginCheckResult {
            profile_id: check.profile_id,
            site: check.site.clone(),
            status,
        })
        .await?;
    info!(profile_id = %check.profile_id, login_check_id = %check.id, ?status, "login check");

    Ok(())
}

// Its own task: a caller that gives up must not leave the check's Tab open or mid re-login.
pub async fn run_check(
    app: &Arc<App>,
    check: &LoginCheck,
    instance_id: &InstanceId,
) -> Result<bool, Error> {
    let (app, check, instance_id) = (Arc::clone(app), check.clone(), *instance_id);

    tokio::spawn(async move { run_check_task(&app, &check, &instance_id).await })
        .await
        .map_err(internal)?
}

// In a Tab of its own that closes afterwards, unless it is the Instance's last one.
async fn run_check_task(
    app: &Arc<App>,
    check: &LoginCheck,
    instance_id: &InstanceId,
) -> Result<bool, Error> {
    let tab = app.instances.open_tab(instance_id, None).await?;
    let checked = check_in_tab(app, check, instance_id, &tab.id).await;

    // The owner decides: its Tab list is the one that knows whether this is the last Tab.
    if let Err(error) = app
        .instances
        .close_tab_unless_last(instance_id, &tab.id, TabCloseReason::Client)
        .await
    {
        warn!(%error, "login check tab not closed");
    }

    let ok = checked?;
    record(app, check, ok).await?;

    Ok(ok)
}

async fn check_in_tab(
    app: &Arc<App>,
    check: &LoginCheck,
    instance_id: &InstanceId,
    tab_id: &TabId,
) -> Result<bool, Error> {
    let page = app.instances.page(instance_id, tab_id).await?;
    if evaluate(app, instance_id, tab_id, &page, check).await? {
        return Ok(true);
    }
    let (true, Some(credential_id)) = (check.auto_relogin, check.credential_id) else {
        return Ok(false);
    };
    let credential = app
        .store
        .run(move |c| credential::get(c, &credential_id))
        .await?;
    let key = BackoffKey {
        url: check.url.clone(),
        expect_selector: check.expect_selector.clone(),
        expect_text: check.expect_text.clone(),
        credential_id,
        credential_updated_at: credential.updated_at,
    };
    if !app.login_checks.relogin_allowed(&check.id, &key) {
        return Ok(false);
    }

    let tab = TabCall {
        instance_id: *instance_id,
        tab_id: *tab_id,
        profile_id: Some(check.profile_id),
        page: page.clone(),
    };
    // The login's own success rule calls check_site; this run records and publishes once.
    let quiet = Relogging::start(app, tab.tab_id);
    let relogin = login(app, &tab, &credential_id, RELOGIN_TIMEOUT).await;
    drop(quiet);

    // A login on the check's own site passed that site's check already.
    let ok = match relogin {
        Err(_) => false,
        Ok(_) if credential.site == check.site => true,
        Ok(_) => evaluate(app, instance_id, tab_id, &page, check).await?,
    };
    if let Some(failures) = app
        .login_checks
        .relogin_outcome(&check.id, &check.profile_id, key, ok)
    {
        warn!(profile_id = %check.profile_id, login_check_id = %check.id, "auto re-login suspended");
        app.events
            .publish(EventData::LoginReloginSuspended {
                profile_id: check.profile_id,
                site: check.site.clone(),
                failures,
            })
            .await?;
    }

    Ok(ok)
}
