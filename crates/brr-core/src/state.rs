use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    Queued,
    Starting,
    Running,
    Idle,
    Reaping,
    Stopping,
    Stopped,
    Killed,
    Crashed,
    Failed,
    Attaching,
    Attached,
    Detached,
    Lost,
}

impl InstanceState {
    pub fn is_final(self) -> bool {
        matches!(
            self,
            InstanceState::Stopped
                | InstanceState::Failed
                | InstanceState::Detached
                | InstanceState::Lost
        )
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum InstanceTrigger {
    Launch,
    Admitted,
    QueueTimeout,
    Cancel,
    Ready,
    LaunchError,
    StartTimeout,
    Stop,
    IdleTimeout,
    Activity,
    Reap,
    GraceOver,
    Evict,
    MemoryPressure,
    LastTabClosed,
    Exited,
    StopTimeout,
    TreeKilled,
    UnexpectedExit,
    AutoRestart,
    NoRestart,
    Reconcile,
    Attach,
    Connected,
    AttachError,
    Detach,
    ConnectionLost,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TabState {
    Opening,
    Loading,
    Ready,
    Idle,
    Reaping,
    AwaitingHuman,
    Crashed,
    Closing,
    Closed,
}

impl TabState {
    pub fn is_persisted(self) -> bool {
        matches!(
            self,
            TabState::Opening | TabState::Closed | TabState::Crashed | TabState::AwaitingHuman
        )
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TabTrigger {
    Open,
    TargetAttached,
    Loaded,
    Navigation,
    IdleTimeout,
    Activity,
    Reap,
    GraceOver,
    CaptchaNeedsHuman,
    DialogBlocking,
    HumanDone,
    HumanTimeout,
    RendererCrash,
    Reload,
    Close,
    InstanceStopping,
    TargetDestroyed,
}
