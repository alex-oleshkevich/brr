use brr_core::{
    AdmissionMode, Display, Instance, InstanceId, InstanceKind, InstanceState, Tab, TabId,
    TabState, TargetId,
};

use super::{Pick, pick};
use crate::services::instances::InstanceView;

const T0: i64 = 100;
const T1: i64 = 200;
const T2: i64 = 300;

fn instance(state: InstanceState, last_activity_at: i64) -> Instance {
    Instance {
        id: InstanceId::new(),
        kind: InstanceKind::Managed,
        profile_id: None,
        build_id: None,
        state,
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
        last_activity_at,
        queued_at: 0,
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
    }
}

fn tab(instance: &Instance, state: TabState, last_activity_at: i64, pinned: bool) -> Tab {
    Tab {
        id: TabId::new(),
        instance_id: instance.id,
        target_id: TargetId("target".to_owned()),
        state,
        url: "about:blank".to_owned(),
        title: String::new(),
        opened_by_brr: true,
        pinned,
        last_activity_at,
        created_at: 0,
        closed_at: None,
        lease: None,
    }
}

fn view(instance: Instance, tabs: Vec<Tab>, pinned: bool) -> InstanceView {
    InstanceView {
        instance,
        tabs,
        pinned,
    }
}

// An Instance with the four Tab candidates of the spec, plus a lone-Tab Instance and an idle one.
struct Fixture {
    views: Vec<InstanceView>,
    oldest_idle: Tab,
    reaping: Tab,
}

fn fixture() -> Fixture {
    let busy = instance(InstanceState::Running, T2);
    let oldest_idle = tab(&busy, TabState::Idle, T1, false);
    let reaping = tab(&busy, TabState::Reaping, T2, false);
    let pinned = tab(&busy, TabState::Idle, T0, true);
    let tabs = vec![oldest_idle.clone(), reaping.clone(), pinned];

    let lonely = instance(InstanceState::Running, T0);
    let last_tab = tab(&lonely, TabState::Idle, T0, false);

    let idle_instance = instance(InstanceState::Idle, T0);

    Fixture {
        views: vec![
            view(busy, tabs, false),
            view(lonely, vec![last_tab], false),
            view(idle_instance, Vec::new(), false),
        ],
        oldest_idle,
        reaping,
    }
}

#[test]
fn pick_takes_the_least_recently_active_tab_first() {
    let fixture = fixture();

    assert_eq!(
        pick(&fixture.views),
        Some(Pick::CloseTab {
            instance_id: fixture.oldest_idle.instance_id,
            tab_id: fixture.oldest_idle.id
        })
    );
}

#[test]
fn pick_then_takes_a_reaping_tab() {
    let mut fixture = fixture();
    fixture.views[0]
        .tabs
        .retain(|tab| tab.id != fixture.oldest_idle.id);

    assert_eq!(
        pick(&fixture.views),
        Some(Pick::CloseTab {
            instance_id: fixture.reaping.instance_id,
            tab_id: fixture.reaping.id
        })
    );
}

#[test]
fn pick_falls_back_to_an_idle_instance_and_never_the_last_tab() {
    let mut fixture = fixture();
    fixture.views[0].tabs.clear();
    let idle_id = fixture.views[2].instance.id;

    assert_eq!(
        pick(&fixture.views),
        Some(Pick::StopInstance {
            instance_id: idle_id
        })
    );
}

#[test]
fn pick_leaves_pinned_subjects_alone() {
    let mut fixture = fixture();
    fixture.views[0].tabs.clear();
    fixture.views[2].pinned = true;

    assert_eq!(pick(&fixture.views), None);
}
