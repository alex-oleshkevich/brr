use brr_core::{
    AdmissionMode, Display, IdleOverrides, Instance, InstanceId, InstanceKind, InstanceState, Tab,
    TabId, TabState, TargetId, settings::IdleSettings,
};

use super::{Due, Grace, TabReap, may_reap_instance, next_due, tab_reap, timeouts};
use crate::services::instances::InstanceView;

const T0: i64 = 1_000_000;
const MIN: i64 = 60_000;

fn tab(state: TabState, last_activity_at: i64) -> Tab {
    let instance = instance();

    Tab {
        id: TabId::new(),
        instance_id: instance.id,
        target_id: TargetId("target".to_owned()),
        state,
        url: "about:blank".to_owned(),
        title: String::new(),
        opened_by_brr: true,
        pinned: false,
        last_activity_at,
        created_at: 0,
        closed_at: None,
        lease: None,
    }
}

fn instance() -> Instance {
    Instance {
        id: InstanceId::new(),
        kind: InstanceKind::Managed,
        profile_id: None,
        build_id: None,
        state: InstanceState::Running,
        display: Display::Headless,
        priority: 0,
        admission: AdmissionMode::Queue,
        pid: None,
        log_path: None,
        cdp_url: None,
        data_dir: None,
        ephemeral: true,
        promote_to_name: None,
        restart_count: 0,
        last_activity_at: T0,
        queued_at: 0,
        started_at: None,
        ended_at: None,
        end_reason: None,
        error: None,
        profile_name: None,
        settings: None,
        template_id: None,
        template_name: None,
        extensions: None,
        proxy_id: None,
        sandbox: None,
        traffic: None,
        host_id: None,
        build: None,
        tab_count: 0,
        lease_count: 0,
        rss_bytes: None,
        capabilities: Vec::new(),
    }
}

fn view(tabs: Vec<Tab>) -> InstanceView {
    InstanceView {
        instance: instance(),
        tabs,
        pinned: false,
    }
}

fn global() -> IdleSettings {
    IdleSettings {
        tab_min: 15,
        instance_min: 30,
        grace_s: 60,
    }
}

#[test]
fn next_due_is_the_earliest_deadline() {
    let first = tab(TabState::Ready, T0);
    let second = tab(TabState::Ready, T0 + 5 * MIN);
    let first_id = first.id;
    let view = view(vec![first, second]);

    let t = timeouts(&global(), &IdleOverrides::default());
    assert_eq!(
        next_due(&view, &Grace::default(), &t),
        Some((T0 + 15 * MIN, Due::TabIdle(first_id)))
    );

    let overrides = IdleOverrides {
        instance_min: Some(1),
        ..IdleOverrides::default()
    };
    let t = timeouts(&global(), &overrides);
    assert_eq!(
        next_due(&view, &Grace::default(), &t),
        Some((T0 + MIN, Due::InstanceIdle))
    );
}

#[test]
fn attached_live_instance_reaps_only_tabs_brr_opened() {
    let mut users = tab(TabState::Ready, T0);
    users.opened_by_brr = false;
    let ours = tab(TabState::Ready, T0 + 5 * MIN);
    let ours_id = ours.id;
    let mut view = view(vec![users, ours]);
    view.instance.kind = InstanceKind::Live;
    view.instance.state = InstanceState::Attached;

    let t = timeouts(&global(), &IdleOverrides::default());
    assert_eq!(
        next_due(&view, &Grace::default(), &t),
        Some((T0 + 20 * MIN, Due::TabIdle(ours_id)))
    );
}

#[test]
fn next_due_follows_grace_and_skips_idle_subjects() {
    let reaping = tab(TabState::Reaping, T0);
    let idle = tab(TabState::Idle, T0);
    let reaping_id = reaping.id;
    let mut view = view(vec![reaping, idle]);
    view.instance.state = InstanceState::Idle;
    let t = timeouts(&global(), &IdleOverrides::default());

    assert_eq!(next_due(&view, &Grace::default(), &t), None);

    let mut grace = Grace::default();
    grace.tabs.insert(reaping_id, T0 + 60_000);
    assert_eq!(
        next_due(&view, &grace, &t),
        Some((T0 + 60_000, Due::TabGraceOver(reaping_id)))
    );

    view.instance.state = InstanceState::Reaping;
    grace.instance = Some(T0 + 10_000);
    assert_eq!(
        next_due(&view, &grace, &t),
        Some((T0 + 10_000, Due::InstanceGraceOver))
    );
}

#[test]
fn reap_rule_blanks_the_last_tab_and_keeps_pinned_and_foreign_tabs() {
    let only = tab(TabState::Idle, T0);
    let only_id = only.id;
    assert_eq!(tab_reap(&view(vec![only]).tabs, &only_id), TabReap::Blank);

    let mut pinned = tab(TabState::Idle, T0);
    pinned.pinned = true;
    let mut foreign = tab(TabState::Idle, T0);
    foreign.opened_by_brr = false;
    let plain = tab(TabState::Idle, T0);
    let ids = (pinned.id, foreign.id, plain.id);
    let view = view(vec![pinned, foreign, plain]);

    assert_eq!(tab_reap(&view.tabs, &ids.0), TabReap::Keep);
    assert_eq!(tab_reap(&view.tabs, &ids.1), TabReap::Keep);
    assert_eq!(tab_reap(&view.tabs, &ids.2), TabReap::Reap);
}

#[test]
fn reap_rule_spares_pinned_and_live_instances() {
    let mut view = view(Vec::new());
    assert!(may_reap_instance(&view.instance, view.pinned));

    view.pinned = true;
    assert!(!may_reap_instance(&view.instance, view.pinned));

    view.pinned = false;
    view.instance.kind = InstanceKind::Live;
    assert!(!may_reap_instance(&view.instance, view.pinned));
}
