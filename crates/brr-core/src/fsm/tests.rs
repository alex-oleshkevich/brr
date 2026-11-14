use test_case::test_case;

use super::{INSTANCE_TRANSITIONS, TAB_TRANSITIONS};
use crate::{Error, InstanceState, InstanceTrigger, TabState, TabTrigger};

const ALL_INSTANCE_STATES: [InstanceState; 14] = [
    InstanceState::Queued,
    InstanceState::Starting,
    InstanceState::Running,
    InstanceState::Idle,
    InstanceState::Reaping,
    InstanceState::Stopping,
    InstanceState::Stopped,
    InstanceState::Killed,
    InstanceState::Crashed,
    InstanceState::Failed,
    InstanceState::Attaching,
    InstanceState::Attached,
    InstanceState::Detached,
    InstanceState::Lost,
];
const MANAGED_NON_FINAL: [InstanceState; 8] = [
    InstanceState::Queued,
    InstanceState::Starting,
    InstanceState::Running,
    InstanceState::Idle,
    InstanceState::Reaping,
    InstanceState::Stopping,
    InstanceState::Killed,
    InstanceState::Crashed,
];
const ALL_TAB_TRIGGERS: [TabTrigger; 17] = [
    TabTrigger::Open,
    TabTrigger::TargetAttached,
    TabTrigger::Loaded,
    TabTrigger::Navigation,
    TabTrigger::IdleTimeout,
    TabTrigger::Activity,
    TabTrigger::Reap,
    TabTrigger::GraceOver,
    TabTrigger::CaptchaNeedsHuman,
    TabTrigger::DialogBlocking,
    TabTrigger::HumanDone,
    TabTrigger::HumanTimeout,
    TabTrigger::RendererCrash,
    TabTrigger::Reload,
    TabTrigger::Close,
    TabTrigger::InstanceStopping,
    TabTrigger::TargetDestroyed,
];
const TAB_CLOSABLE: [TabState; 7] = [
    TabState::Opening,
    TabState::Reaping,
    TabState::Crashed,
    TabState::Ready,
    TabState::Idle,
    TabState::Loading,
    TabState::AwaitingHuman,
];

#[test]
fn every_listed_instance_pair_lands_on_the_trigger_target() {
    for &(from, trigger) in INSTANCE_TRANSITIONS {
        assert_eq!(from.transition(trigger).unwrap(), trigger.target());
    }
}

#[test]
fn stop_on_stopped_is_invalid() {
    let error = InstanceState::Stopped
        .transition(InstanceTrigger::Stop)
        .unwrap_err();

    assert_eq!(
        error,
        Error::InvalidTransition {
            from: "stopped".to_string(),
            to: "stopping".to_string()
        }
    );
}

#[test_case(InstanceState::Queued, InstanceTrigger::Cancel, InstanceState::Stopped; "cancel queued")]
#[test_case(InstanceState::Queued, InstanceTrigger::LaunchError, InstanceState::Failed; "setup failure while queued")]
#[test_case(InstanceState::Idle, InstanceTrigger::Evict, InstanceState::Stopping; "evict idle")]
#[test_case(InstanceState::Reaping, InstanceTrigger::UnexpectedExit, InstanceState::Crashed; "crash while reaping")]
#[test_case(InstanceState::Crashed, InstanceTrigger::AutoRestart, InstanceState::Starting; "restart crashed")]
#[test_case(InstanceState::Attaching, InstanceTrigger::AttachError, InstanceState::Failed; "attach error")]
#[test_case(InstanceState::Attaching, InstanceTrigger::Detach, InstanceState::Detached; "detach while attaching")]
#[test_case(InstanceState::Attached, InstanceTrigger::ConnectionLost, InstanceState::Lost; "connection lost")]
fn instance_transition_reaches(from: InstanceState, trigger: InstanceTrigger, to: InstanceState) {
    assert_eq!(from.transition(trigger).unwrap(), to);
}

#[test]
fn reconcile_stops_every_non_final_managed_state() {
    for from in MANAGED_NON_FINAL {
        assert_eq!(
            from.transition(InstanceTrigger::Reconcile).unwrap(),
            InstanceState::Stopped
        );
    }
}

#[test]
fn reconcile_is_invalid_from_final_states() {
    for from in ALL_INSTANCE_STATES
        .into_iter()
        .filter(|state| state.is_final())
    {
        assert!(
            from.transition(InstanceTrigger::Reconcile).is_err(),
            "{from:?}"
        );
    }
}

#[test]
fn reconcile_is_the_only_trigger_from_running_to_stopped() {
    assert_eq!(
        InstanceState::Running
            .transition(InstanceTrigger::Exited)
            .unwrap_err(),
        Error::InvalidTransition {
            from: "running".to_string(),
            to: "stopped".to_string()
        }
    );
}

#[test]
fn every_listed_tab_pair_lands_on_the_trigger_target() {
    for &(from, trigger) in TAB_TRANSITIONS {
        assert_eq!(from.transition(trigger).unwrap(), trigger.target());
    }
}

#[test]
fn close_and_instance_stopping_close_every_non_final_tab_state() {
    for from in TAB_CLOSABLE {
        for trigger in [TabTrigger::Close, TabTrigger::InstanceStopping] {
            assert_eq!(from.transition(trigger).unwrap(), TabState::Closing);
        }
    }
}

#[test]
fn closed_tab_is_terminal() {
    for trigger in ALL_TAB_TRIGGERS {
        assert!(TabState::Closed.transition(trigger).is_err(), "{trigger:?}");
    }
}
