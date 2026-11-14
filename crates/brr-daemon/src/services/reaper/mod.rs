use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use crate::{
    app::App,
    services::{
        LIMITS_CHANGED, MIN_CHECK_S, SETTINGS_CHANGED, STATE_CHANGED,
        events::Published,
        host,
        instances::{
            InstanceView,
            idle::{TabReap, may_reap_instance, tab_reap},
        },
        until,
    },
};
use brr_core::{
    EndReason, Error, EventData, InstanceId, InstanceKind, InstanceState, TabCloseReason, TabId,
    TabState, settings::LimitMode,
};
use tokio::{
    sync::broadcast::{self, error::RecvError},
    time::Instant,
};
use tracing::{debug, warn};

#[cfg(test)]
mod tests;

const CLOSE_TAB: &str = "close_tab";
const STOP_INSTANCE: &str = "stop_instance";

#[derive(Debug, PartialEq, Eq)]
pub enum Pick {
    CloseTab {
        instance_id: InstanceId,
        tab_id: TabId,
    },
    StopInstance {
        instance_id: InstanceId,
    },
}

pub fn spawn(app: Arc<App>) {
    // Held weakly: a strong reference would keep the daemon lock after the last owner drops the App.
    let events = app.events.subscribe();
    tokio::spawn(run(Arc::downgrade(&app), events));
}

// A3: idle Tabs go before idle Instances, least recently active first. Only idle or reaping
// subjects qualify, which already means no Lease activity for the idle timeout.
pub fn pick(views: &[InstanceView]) -> Option<Pick> {
    let idle_tab = |state: TabState| matches!(state, TabState::Idle | TabState::Reaping);

    let tab = views
        .iter()
        .flat_map(|view| {
            view.tabs
                .iter()
                .filter(|tab| idle_tab(tab.state))
                .filter(|tab| tab_reap(&view.tabs, &tab.id) == TabReap::Reap)
        })
        .min_by_key(|tab| tab.last_activity_at);
    if let Some(tab) = tab {
        return Some(Pick::CloseTab {
            instance_id: tab.instance_id,
            tab_id: tab.id,
        });
    }

    views
        .iter()
        .filter(|view| {
            matches!(
                view.instance.state,
                InstanceState::Idle | InstanceState::Reaping
            ) && view.instance.kind == InstanceKind::Managed
                && may_reap_instance(&view.instance, view.pinned)
        })
        .min_by_key(|view| view.instance.last_activity_at)
        .map(|view| Pick::StopInstance {
            instance_id: view.instance.id,
        })
}

enum Wake {
    Settings,
    // An Instance changed state: the tick is armed or disarmed by whether any exists.
    Instances,
    Closed,
}

// Static mode sleeps until a settings event; adaptive mode ticks every pressure_check_s, and only
// while an Instance exists, since there is nothing to relieve otherwise.
async fn run(app: Weak<App>, mut events: broadcast::Receiver<Arc<Published>>) {
    let mut tick_at: Option<Instant> = None;

    loop {
        let Some(strong) = app.upgrade() else {
            return;
        };
        let settings = strong.settings();
        let running = !strong.instances.views().is_empty();
        drop(strong);
        if settings.limits.mode == LimitMode::Adaptive && running {
            let check_s = settings.adaptive.pressure_check_s.max(MIN_CHECK_S);
            tick_at.get_or_insert_with(|| Instant::now() + Duration::from_secs(check_s));
        } else {
            tick_at = None;
        }

        tokio::select! {
            () = until(tick_at) => {
                tick_at = None;
                let Some(strong) = app.upgrade() else {
                    return;
                };
                relieve(&strong).await;
            }
            wake = next_wake(&mut events) => match wake {
                Wake::Closed => return,
                Wake::Settings => tick_at = None,
                Wake::Instances => {}
            },
        }
    }
}

// Other events are skipped without re-reading any setting.
async fn next_wake(events: &mut broadcast::Receiver<Arc<Published>>) -> Wake {
    loop {
        match events.recv().await {
            Ok(published) => match published.event.kind.as_str() {
                LIMITS_CHANGED | SETTINGS_CHANGED => return Wake::Settings,
                STATE_CHANGED => return Wake::Instances,
                _ => {}
            },
            Err(RecvError::Lagged(_)) => return Wake::Settings,
            Err(RecvError::Closed) => return Wake::Closed,
        }
    }
}

// One action per tick, so the next tick measures its effect.
async fn relieve(app: &Arc<App>) {
    let min_free_mb = app.settings().adaptive.min_free_ram_mb;
    let free_ram_mb = match host::memory() {
        Ok(memory) => memory.available_mb,
        Err(error) => {
            warn!(%error, "memory pressure not checked");

            return;
        }
    };
    if free_ram_mb >= min_free_mb {
        return;
    }
    let Some(pick) = pick(&app.instances.views()) else {
        return;
    };

    let (action, target_id, acted) = match &pick {
        Pick::CloseTab {
            instance_id,
            tab_id,
        } => (
            CLOSE_TAB,
            tab_id.to_string(),
            app.instances
                .close_tab(instance_id, tab_id, TabCloseReason::Reaped)
                .await
                .map(|_| ()),
        ),
        Pick::StopInstance { instance_id } => (
            STOP_INSTANCE,
            instance_id.to_string(),
            app.instances
                .stop(app, instance_id, EndReason::MemoryPressure)
                .await
                .map(|_| ()),
        ),
    };

    match acted {
        // Activity won the race: the subject is no longer idle and stays.
        Err(Error::InvalidTransition { .. }) => {
            debug!(action, %target_id, "pressure pick not idle any more")
        }
        Err(error) => warn!(action, %target_id, %error, "pressure action failed"),
        Ok(()) => {
            warn!(free_ram_mb, action, %target_id, "memory pressure: reclaimed an idle subject");
            let published = app
                .events
                .publish(EventData::MemoryPressure {
                    free_ram_mb,
                    action: action.to_owned(),
                    target_id,
                })
                .await;
            if let Err(error) = published {
                warn!(%error, "memory pressure event not published");
            }
        }
    }
}
