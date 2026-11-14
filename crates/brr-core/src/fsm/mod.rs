use serde::Serialize;

use crate::{Error, InstanceState, InstanceTrigger, TabState, TabTrigger};

#[cfg(test)]
mod tests;

const INSTANCE_TRANSITIONS: &[(InstanceState, InstanceTrigger)] = &[
    (InstanceState::Queued, InstanceTrigger::Admitted),
    (InstanceState::Queued, InstanceTrigger::QueueTimeout),
    (InstanceState::Queued, InstanceTrigger::Cancel),
    (InstanceState::Queued, InstanceTrigger::LaunchError),
    (InstanceState::Starting, InstanceTrigger::Ready),
    (InstanceState::Starting, InstanceTrigger::LaunchError),
    (InstanceState::Starting, InstanceTrigger::StartTimeout),
    (InstanceState::Starting, InstanceTrigger::Stop),
    (InstanceState::Running, InstanceTrigger::IdleTimeout),
    (InstanceState::Running, InstanceTrigger::Stop),
    (InstanceState::Running, InstanceTrigger::LastTabClosed),
    (InstanceState::Running, InstanceTrigger::UnexpectedExit),
    (InstanceState::Idle, InstanceTrigger::Activity),
    (InstanceState::Idle, InstanceTrigger::Reap),
    (InstanceState::Idle, InstanceTrigger::Stop),
    (InstanceState::Idle, InstanceTrigger::Evict),
    (InstanceState::Idle, InstanceTrigger::MemoryPressure),
    (InstanceState::Idle, InstanceTrigger::UnexpectedExit),
    (InstanceState::Reaping, InstanceTrigger::Activity),
    (InstanceState::Reaping, InstanceTrigger::GraceOver),
    (InstanceState::Reaping, InstanceTrigger::Stop),
    (InstanceState::Reaping, InstanceTrigger::UnexpectedExit),
    (InstanceState::Stopping, InstanceTrigger::Exited),
    (InstanceState::Stopping, InstanceTrigger::StopTimeout),
    (InstanceState::Killed, InstanceTrigger::TreeKilled),
    (InstanceState::Crashed, InstanceTrigger::AutoRestart),
    (InstanceState::Crashed, InstanceTrigger::NoRestart),
    (InstanceState::Attaching, InstanceTrigger::Connected),
    (InstanceState::Attaching, InstanceTrigger::AttachError),
    (InstanceState::Attaching, InstanceTrigger::Detach),
    (InstanceState::Attached, InstanceTrigger::Detach),
    (InstanceState::Attached, InstanceTrigger::ConnectionLost),
    (InstanceState::Queued, InstanceTrigger::Reconcile),
    (InstanceState::Starting, InstanceTrigger::Reconcile),
    (InstanceState::Running, InstanceTrigger::Reconcile),
    (InstanceState::Idle, InstanceTrigger::Reconcile),
    (InstanceState::Reaping, InstanceTrigger::Reconcile),
    (InstanceState::Stopping, InstanceTrigger::Reconcile),
    (InstanceState::Killed, InstanceTrigger::Reconcile),
    (InstanceState::Crashed, InstanceTrigger::Reconcile),
];

const TAB_TRANSITIONS: &[(TabState, TabTrigger)] = &[
    (TabState::Opening, TabTrigger::TargetAttached),
    (TabState::Opening, TabTrigger::Close),
    (TabState::Opening, TabTrigger::InstanceStopping),
    (TabState::Loading, TabTrigger::Loaded),
    (TabState::Loading, TabTrigger::CaptchaNeedsHuman),
    (TabState::Loading, TabTrigger::DialogBlocking),
    (TabState::Loading, TabTrigger::RendererCrash),
    (TabState::Loading, TabTrigger::Close),
    (TabState::Loading, TabTrigger::InstanceStopping),
    (TabState::Ready, TabTrigger::Navigation),
    (TabState::Ready, TabTrigger::IdleTimeout),
    (TabState::Ready, TabTrigger::CaptchaNeedsHuman),
    (TabState::Ready, TabTrigger::DialogBlocking),
    (TabState::Ready, TabTrigger::RendererCrash),
    (TabState::Ready, TabTrigger::Close),
    (TabState::Ready, TabTrigger::InstanceStopping),
    (TabState::Idle, TabTrigger::Activity),
    (TabState::Idle, TabTrigger::Reap),
    (TabState::Idle, TabTrigger::Close),
    (TabState::Idle, TabTrigger::InstanceStopping),
    (TabState::Reaping, TabTrigger::Activity),
    (TabState::Reaping, TabTrigger::GraceOver),
    (TabState::Reaping, TabTrigger::Close),
    (TabState::Reaping, TabTrigger::InstanceStopping),
    (TabState::AwaitingHuman, TabTrigger::HumanDone),
    (TabState::AwaitingHuman, TabTrigger::HumanTimeout),
    (TabState::AwaitingHuman, TabTrigger::Close),
    (TabState::AwaitingHuman, TabTrigger::InstanceStopping),
    (TabState::Crashed, TabTrigger::Reload),
    (TabState::Crashed, TabTrigger::Close),
    (TabState::Crashed, TabTrigger::InstanceStopping),
    (TabState::Closing, TabTrigger::TargetDestroyed),
];

impl InstanceTrigger {
    pub fn target(self) -> InstanceState {
        match self {
            InstanceTrigger::Launch => InstanceState::Queued,
            InstanceTrigger::Admitted | InstanceTrigger::AutoRestart => InstanceState::Starting,
            InstanceTrigger::QueueTimeout
            | InstanceTrigger::Cancel
            | InstanceTrigger::Exited
            | InstanceTrigger::TreeKilled
            | InstanceTrigger::NoRestart
            | InstanceTrigger::Reconcile => InstanceState::Stopped,
            InstanceTrigger::Ready | InstanceTrigger::Activity => InstanceState::Running,
            InstanceTrigger::LaunchError
            | InstanceTrigger::StartTimeout
            | InstanceTrigger::AttachError => InstanceState::Failed,
            InstanceTrigger::Stop
            | InstanceTrigger::GraceOver
            | InstanceTrigger::Evict
            | InstanceTrigger::MemoryPressure
            | InstanceTrigger::LastTabClosed => InstanceState::Stopping,
            InstanceTrigger::IdleTimeout => InstanceState::Idle,
            InstanceTrigger::Reap => InstanceState::Reaping,
            InstanceTrigger::StopTimeout => InstanceState::Killed,
            InstanceTrigger::UnexpectedExit => InstanceState::Crashed,
            InstanceTrigger::Attach => InstanceState::Attaching,
            InstanceTrigger::Connected => InstanceState::Attached,
            InstanceTrigger::Detach => InstanceState::Detached,
            InstanceTrigger::ConnectionLost => InstanceState::Lost,
        }
    }
}

impl InstanceState {
    pub fn transition(self, trigger: InstanceTrigger) -> Result<InstanceState, Error> {
        if INSTANCE_TRANSITIONS.contains(&(self, trigger)) {
            Ok(trigger.target())
        } else {
            Err(invalid_transition(self, trigger.target()))
        }
    }
}

impl TabTrigger {
    pub fn target(self) -> TabState {
        match self {
            TabTrigger::Open => TabState::Opening,
            TabTrigger::TargetAttached | TabTrigger::Navigation | TabTrigger::Reload => {
                TabState::Loading
            }
            TabTrigger::Loaded
            | TabTrigger::Activity
            | TabTrigger::HumanDone
            | TabTrigger::HumanTimeout => TabState::Ready,
            TabTrigger::IdleTimeout => TabState::Idle,
            TabTrigger::Reap => TabState::Reaping,
            TabTrigger::CaptchaNeedsHuman | TabTrigger::DialogBlocking => TabState::AwaitingHuman,
            TabTrigger::RendererCrash => TabState::Crashed,
            TabTrigger::GraceOver | TabTrigger::Close | TabTrigger::InstanceStopping => {
                TabState::Closing
            }
            TabTrigger::TargetDestroyed => TabState::Closed,
        }
    }
}

impl TabState {
    pub fn transition(self, trigger: TabTrigger) -> Result<TabState, Error> {
        if TAB_TRANSITIONS.contains(&(self, trigger)) {
            Ok(trigger.target())
        } else {
            Err(invalid_transition(self, trigger.target()))
        }
    }
}

fn invalid_transition(from: impl Serialize, to: impl Serialize) -> Error {
    Error::InvalidTransition {
        from: snake_case_name(from),
        to: snake_case_name(to),
    }
}

fn snake_case_name(state: impl Serialize) -> String {
    match serde_json::to_value(state) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}
