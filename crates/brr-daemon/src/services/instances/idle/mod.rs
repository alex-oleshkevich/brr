use std::collections::BTreeMap;

use brr_core::{
    IdleOverrides, Instance, InstanceKind, InstanceState, Tab, TabId, TabState,
    settings::IdleSettings,
};

use super::InstanceView;

#[cfg(test)]
mod tests;

const MS_PER_MIN: i64 = 60_000;
const MS_PER_S: i64 = 1_000;

pub struct Timeouts {
    pub tab_ms: i64,
    pub instance_ms: i64,
    pub grace_ms: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Due {
    InstanceIdle,
    InstanceGraceOver,
    TabIdle(TabId),
    TabGraceOver(TabId),
}

#[derive(Debug, PartialEq, Eq)]
pub enum TabReap {
    Reap,
    Blank,
    Keep,
}

// The moment each reaping subject's grace period ends.
#[derive(Default)]
pub struct Grace {
    pub instance: Option<i64>,
    pub tabs: BTreeMap<TabId, i64>,
}

// A Profile's own value wins over the global one.
pub fn timeouts(global: &IdleSettings, profile: &IdleOverrides) -> Timeouts {
    Timeouts {
        tab_ms: i64::from(profile.tab_min.unwrap_or(global.tab_min)) * MS_PER_MIN,
        instance_ms: i64::from(profile.instance_min.unwrap_or(global.instance_min)) * MS_PER_MIN,
        grace_ms: i64::from(profile.grace_s.unwrap_or(global.grace_s)) * MS_PER_S,
    }
}

pub fn next_due(view: &InstanceView, grace: &Grace, t: &Timeouts) -> Option<(i64, Due)> {
    let mut earliest: Option<(i64, Due)> = None;
    let mut offer = |at: i64, due: Due| {
        if earliest.as_ref().is_none_or(|(best, _)| at < *best) {
            earliest = Some((at, due));
        }
    };

    match view.instance.state {
        InstanceState::Running => offer(
            view.instance.last_activity_at + t.instance_ms,
            Due::InstanceIdle,
        ),
        InstanceState::Reaping => {
            if let Some(at) = grace.instance {
                offer(at, Due::InstanceGraceOver);
            }
        }
        _ => {}
    }

    if matches!(
        view.instance.state,
        InstanceState::Running
            | InstanceState::Idle
            | InstanceState::Reaping
            | InstanceState::Attached
    ) {
        for tab in &view.tabs {
            // Only Tabs brr opened in a Live browser are brr's to reap.
            if view.instance.kind == InstanceKind::Live && !tab.opened_by_brr {
                continue;
            }
            match tab.state {
                TabState::Ready => offer(tab.last_activity_at + t.tab_ms, Due::TabIdle(tab.id)),
                TabState::Reaping => {
                    if let Some(at) = grace.tabs.get(&tab.id) {
                        offer(*at, Due::TabGraceOver(tab.id));
                    }
                }
                _ => {}
            }
        }
    }

    earliest
}

pub fn may_reap_instance(instance: &Instance, pinned: bool) -> bool {
    instance.kind == InstanceKind::Managed && !pinned
}

// I2: the last open Tab is blanked instead of closed.
pub fn tab_reap<'a>(tabs: impl IntoIterator<Item = &'a Tab>, tab_id: &TabId) -> TabReap {
    let mut target = None;
    let mut open = 0;
    for tab in tabs {
        if tab.id == *tab_id {
            target = Some(tab);
        }
        if tab.state != TabState::Closing {
            open += 1;
        }
    }

    match target {
        Some(tab) if !tab.pinned && tab.opened_by_brr => {
            if open <= 1 {
                TabReap::Blank
            } else {
                TabReap::Reap
            }
        }
        _ => TabReap::Keep,
    }
}
