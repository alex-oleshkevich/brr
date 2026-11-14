use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    id::{
        ArtifactId, BackupId, BuildId, ClientId, CredentialId, ExtensionId, InstanceId, LeaseId,
        ProfileId, ProxyId, TabId,
    },
    model::{
        AdmissionOutcome, ArtifactKind, BuildSource, ClientEndReason, ClientKind, HumanReason,
        InstallPhase, LoginStatus, ReleaseReason, TabCloseReason, UsdMicros,
    },
    state::{InstanceState, InstanceTrigger, TabState, TabTrigger},
};

const DATA_KEY: &str = "data";

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SubjectType {
    Instance,
    Tab,
    Client,
    Lease,
    Build,
    Profile,
    Extension,
    Artifact,
    Proxy,
    System,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Subject {
    #[serde(rename = "type")]
    pub kind: SubjectType,
    pub id: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Download {
    pub tab_id: TabId,
    pub instance_id: InstanceId,
    pub artifact_id: Option<ArtifactId>,
    pub filename: String,
    pub size: u64,
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct SweepCounts {
    pub artifacts: u32,
    pub builds: u32,
    pub extensions: u32,
    pub events: u32,
    pub backups: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct AdmissionInputs {
    pub free_ram_mb: u64,
    pub reserved_mb: u64,
    pub load_per_core: f64,
    pub counts: BTreeMap<String, u32>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(tag = "kind", content = "data")]
pub enum EventData {
    #[serde(rename = "instance.state_changed")]
    InstanceStateChanged {
        instance_id: InstanceId,
        profile_id: Option<ProfileId>,
        from: Option<InstanceState>,
        to: InstanceState,
        trigger: InstanceTrigger,
        reason: Option<String>,
    },
    #[serde(rename = "instance.queued")]
    InstanceQueued {
        instance_id: InstanceId,
        queue_pos: u32,
        priority: u8,
    },
    #[serde(rename = "instance.reaping_warning")]
    InstanceReapingWarning {
        instance_id: InstanceId,
        grace_ends_at: i64,
    },
    #[serde(rename = "tab.opened")]
    TabOpened {
        tab_id: TabId,
        instance_id: InstanceId,
        url: String,
        opened_by_brr: bool,
    },
    #[serde(rename = "tab.closed")]
    TabClosed {
        tab_id: TabId,
        instance_id: InstanceId,
        reason: TabCloseReason,
    },
    #[serde(rename = "tab.state_changed")]
    TabStateChanged {
        tab_id: TabId,
        instance_id: InstanceId,
        from: Option<TabState>,
        to: TabState,
        trigger: TabTrigger,
    },
    #[serde(rename = "tab.crashed")]
    TabCrashed {
        tab_id: TabId,
        instance_id: InstanceId,
    },
    #[serde(rename = "tab.reaping_warning")]
    TabReapingWarning {
        tab_id: TabId,
        instance_id: InstanceId,
        grace_ends_at: i64,
    },
    #[serde(rename = "tab.awaiting_human")]
    TabAwaitingHuman {
        tab_id: TabId,
        instance_id: InstanceId,
        reason: HumanReason,
        deadline: i64,
    },
    #[serde(rename = "tab.navigated")]
    TabNavigated {
        tab_id: TabId,
        instance_id: InstanceId,
        url: String,
        title: String,
    },
    #[serde(rename = "client.created")]
    ClientCreated {
        client_id: ClientId,
        kind: ClientKind,
        label: String,
    },
    #[serde(rename = "client.ended")]
    ClientEnded {
        client_id: ClientId,
        reason: ClientEndReason,
    },
    #[serde(rename = "lease.acquired")]
    LeaseAcquired {
        lease_id: LeaseId,
        client_id: ClientId,
        instance_id: InstanceId,
        tab_id: Option<TabId>,
        exclusive: bool,
    },
    #[serde(rename = "lease.released")]
    LeaseReleased {
        lease_id: LeaseId,
        client_id: ClientId,
        instance_id: InstanceId,
        tab_id: Option<TabId>,
        exclusive: bool,
        reason: ReleaseReason,
    },
    #[serde(rename = "lease.transferred")]
    LeaseTransferred {
        lease_id: LeaseId,
        instance_id: InstanceId,
        from_client_id: ClientId,
        to_client_id: ClientId,
    },
    #[serde(rename = "admission.decided")]
    AdmissionDecided {
        instance_id: InstanceId,
        outcome: AdmissionOutcome,
        inputs: AdmissionInputs,
    },
    #[serde(rename = "system.memory_pressure")]
    MemoryPressure {
        free_ram_mb: u64,
        action: String,
        target_id: String,
    },
    #[serde(rename = "build.install_progress")]
    BuildInstallProgress {
        build_id: BuildId,
        bytes: u64,
        total: Option<u64>,
        phase: InstallPhase,
    },
    #[serde(rename = "build.installed")]
    BuildInstalled {
        build_id: BuildId,
        source: BuildSource,
        version: String,
    },
    #[serde(rename = "build.install_failed")]
    BuildInstallFailed {
        build_id: BuildId,
        source: BuildSource,
        version: String,
        error: String,
    },
    #[serde(rename = "build.removed")]
    BuildRemoved {
        build_id: BuildId,
        source: BuildSource,
        version: String,
    },
    #[serde(rename = "build.channel_moved")]
    BuildChannelMoved {
        source: BuildSource,
        channel: String,
        from: Option<String>,
        to: String,
    },
    #[serde(rename = "profile.created")]
    ProfileCreated { profile_id: ProfileId, name: String },
    #[serde(rename = "profile.updated")]
    ProfileUpdated { profile_id: ProfileId, name: String },
    #[serde(rename = "profile.deleted")]
    ProfileDeleted { profile_id: ProfileId, name: String },
    #[serde(rename = "extension.updated")]
    ExtensionUpdated {
        extension_id: ExtensionId,
        from: Option<String>,
        to: String,
    },
    #[serde(rename = "download.started")]
    DownloadStarted(Download),
    #[serde(rename = "download.completed")]
    DownloadCompleted(Download),
    #[serde(rename = "download.cancelled")]
    DownloadCancelled(Download),
    #[serde(rename = "artifact.created")]
    ArtifactCreated {
        artifact_id: ArtifactId,
        kind: ArtifactKind,
        size: u64,
    },
    #[serde(rename = "artifact.deleted")]
    ArtifactDeleted {
        artifact_id: ArtifactId,
        kind: ArtifactKind,
        size: u64,
        reason: Option<String>,
    },
    #[serde(rename = "retention.sweep_completed")]
    SweepCompleted {
        dry_run: bool,
        deleted: SweepCounts,
        bytes: u64,
    },
    #[serde(rename = "backup.completed")]
    BackupCompleted {
        profile_id: ProfileId,
        backup_id: Option<BackupId>,
    },
    #[serde(rename = "backup.failed")]
    BackupFailed {
        profile_id: ProfileId,
        error: String,
    },
    #[serde(rename = "backup.skipped")]
    BackupSkipped { profile_id: ProfileId },
    #[serde(rename = "captcha.detected")]
    CaptchaDetected {
        tab_id: TabId,
        instance_id: InstanceId,
        site: String,
        #[serde(rename = "type")]
        captcha_type: String,
    },
    #[serde(rename = "captcha.solved")]
    CaptchaSolved {
        tab_id: TabId,
        instance_id: InstanceId,
        site: String,
        #[serde(rename = "type")]
        captcha_type: String,
        cost_usd: UsdMicros,
        duration_ms: u64,
    },
    #[serde(rename = "captcha.failed")]
    CaptchaFailed {
        tab_id: TabId,
        instance_id: InstanceId,
        site: String,
        #[serde(rename = "type")]
        captcha_type: String,
        error: String,
    },
    #[serde(rename = "captcha.budget_exhausted")]
    CaptchaBudgetExhausted { spent_usd: UsdMicros, solves: u32 },
    #[serde(rename = "login.performed")]
    LoginPerformed {
        tab_id: TabId,
        instance_id: InstanceId,
        credential_id: CredentialId,
        ok: bool,
        step: Option<u8>,
    },
    #[serde(rename = "login.check_result")]
    LoginCheckResult {
        profile_id: ProfileId,
        site: String,
        status: LoginStatus,
    },
    #[serde(rename = "login.relogin_suspended")]
    LoginReloginSuspended {
        profile_id: ProfileId,
        site: String,
        failures: u32,
    },
    #[serde(rename = "proxy.health")]
    ProxyHealth {
        proxy_id: ProxyId,
        healthy: bool,
        exit_ip: Option<String>,
        country: Option<String>,
        latency_ms: Option<u64>,
    },
    #[serde(rename = "limits.changed")]
    LimitsChanged { keys: Vec<String> },
    #[serde(rename = "settings.changed")]
    SettingsChanged { keys: Vec<String> },
}

fn subject(kind: SubjectType, id: impl ToString) -> Subject {
    Subject {
        kind,
        id: id.to_string(),
    }
}

impl EventData {
    pub fn kind(&self) -> &'static str {
        match self {
            EventData::InstanceStateChanged { .. } => "instance.state_changed",
            EventData::InstanceQueued { .. } => "instance.queued",
            EventData::InstanceReapingWarning { .. } => "instance.reaping_warning",
            EventData::TabOpened { .. } => "tab.opened",
            EventData::TabClosed { .. } => "tab.closed",
            EventData::TabStateChanged { .. } => "tab.state_changed",
            EventData::TabCrashed { .. } => "tab.crashed",
            EventData::TabReapingWarning { .. } => "tab.reaping_warning",
            EventData::TabAwaitingHuman { .. } => "tab.awaiting_human",
            EventData::TabNavigated { .. } => "tab.navigated",
            EventData::ClientCreated { .. } => "client.created",
            EventData::ClientEnded { .. } => "client.ended",
            EventData::LeaseAcquired { .. } => "lease.acquired",
            EventData::LeaseReleased { .. } => "lease.released",
            EventData::LeaseTransferred { .. } => "lease.transferred",
            EventData::AdmissionDecided { .. } => "admission.decided",
            EventData::MemoryPressure { .. } => "system.memory_pressure",
            EventData::BuildInstallProgress { .. } => "build.install_progress",
            EventData::BuildInstalled { .. } => "build.installed",
            EventData::BuildInstallFailed { .. } => "build.install_failed",
            EventData::BuildRemoved { .. } => "build.removed",
            EventData::BuildChannelMoved { .. } => "build.channel_moved",
            EventData::ProfileCreated { .. } => "profile.created",
            EventData::ProfileUpdated { .. } => "profile.updated",
            EventData::ProfileDeleted { .. } => "profile.deleted",
            EventData::ExtensionUpdated { .. } => "extension.updated",
            EventData::DownloadStarted(_) => "download.started",
            EventData::DownloadCompleted(_) => "download.completed",
            EventData::DownloadCancelled(_) => "download.cancelled",
            EventData::ArtifactCreated { .. } => "artifact.created",
            EventData::ArtifactDeleted { .. } => "artifact.deleted",
            EventData::SweepCompleted { .. } => "retention.sweep_completed",
            EventData::BackupCompleted { .. } => "backup.completed",
            EventData::BackupFailed { .. } => "backup.failed",
            EventData::BackupSkipped { .. } => "backup.skipped",
            EventData::CaptchaDetected { .. } => "captcha.detected",
            EventData::CaptchaSolved { .. } => "captcha.solved",
            EventData::CaptchaFailed { .. } => "captcha.failed",
            EventData::CaptchaBudgetExhausted { .. } => "captcha.budget_exhausted",
            EventData::LoginPerformed { .. } => "login.performed",
            EventData::LoginCheckResult { .. } => "login.check_result",
            EventData::LoginReloginSuspended { .. } => "login.relogin_suspended",
            EventData::ProxyHealth { .. } => "proxy.health",
            EventData::LimitsChanged { .. } => "limits.changed",
            EventData::SettingsChanged { .. } => "settings.changed",
        }
    }

    pub fn subject(&self) -> Subject {
        match self {
            EventData::InstanceStateChanged { instance_id, .. }
            | EventData::InstanceQueued { instance_id, .. }
            | EventData::InstanceReapingWarning { instance_id, .. }
            | EventData::AdmissionDecided { instance_id, .. } => {
                subject(SubjectType::Instance, instance_id)
            }
            EventData::TabOpened { tab_id, .. }
            | EventData::TabClosed { tab_id, .. }
            | EventData::TabStateChanged { tab_id, .. }
            | EventData::TabCrashed { tab_id, .. }
            | EventData::TabReapingWarning { tab_id, .. }
            | EventData::TabAwaitingHuman { tab_id, .. }
            | EventData::TabNavigated { tab_id, .. }
            | EventData::CaptchaDetected { tab_id, .. }
            | EventData::CaptchaSolved { tab_id, .. }
            | EventData::CaptchaFailed { tab_id, .. }
            | EventData::LoginPerformed { tab_id, .. } => subject(SubjectType::Tab, tab_id),
            EventData::DownloadStarted(download)
            | EventData::DownloadCompleted(download)
            | EventData::DownloadCancelled(download) => subject(SubjectType::Tab, download.tab_id),
            EventData::ClientCreated { client_id, .. }
            | EventData::ClientEnded { client_id, .. } => subject(SubjectType::Client, client_id),
            EventData::LeaseAcquired { lease_id, .. }
            | EventData::LeaseReleased { lease_id, .. }
            | EventData::LeaseTransferred { lease_id, .. } => subject(SubjectType::Lease, lease_id),
            EventData::BuildInstallProgress { build_id, .. }
            | EventData::BuildInstalled { build_id, .. }
            | EventData::BuildInstallFailed { build_id, .. }
            | EventData::BuildRemoved { build_id, .. } => subject(SubjectType::Build, build_id),
            EventData::BuildChannelMoved {
                source, channel, ..
            } => subject(SubjectType::Build, format!("{source}:{channel}")),
            EventData::ProfileCreated { profile_id, .. }
            | EventData::ProfileUpdated { profile_id, .. }
            | EventData::ProfileDeleted { profile_id, .. }
            | EventData::BackupCompleted { profile_id, .. }
            | EventData::BackupFailed { profile_id, .. }
            | EventData::BackupSkipped { profile_id }
            | EventData::LoginCheckResult { profile_id, .. }
            | EventData::LoginReloginSuspended { profile_id, .. } => {
                subject(SubjectType::Profile, profile_id)
            }
            EventData::ExtensionUpdated { extension_id, .. } => {
                subject(SubjectType::Extension, extension_id)
            }
            EventData::ArtifactCreated { artifact_id, .. }
            | EventData::ArtifactDeleted { artifact_id, .. } => {
                subject(SubjectType::Artifact, artifact_id)
            }
            EventData::ProxyHealth { proxy_id, .. } => subject(SubjectType::Proxy, proxy_id),
            EventData::MemoryPressure { .. } => subject(SubjectType::System, "memory"),
            EventData::SweepCompleted { .. } => subject(SubjectType::System, "retention"),
            EventData::CaptchaBudgetExhausted { .. } => subject(SubjectType::System, "captcha"),
            EventData::LimitsChanged { .. } => subject(SubjectType::System, "limits"),
            EventData::SettingsChanged { .. } => subject(SubjectType::System, "settings"),
        }
    }

    pub fn transient(&self) -> bool {
        matches!(
            self,
            EventData::TabStateChanged { .. }
                | EventData::TabNavigated { .. }
                | EventData::BuildInstallProgress { .. }
                | EventData::DownloadStarted(_)
        )
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Event {
    pub seq: i64,
    pub ts: i64,
    pub kind: String,
    pub subject: Subject,
    // The Client whose call caused the event; None for what the daemon does on its own.
    #[serde(default)]
    pub actor_client_id: Option<ClientId>,
    pub data: Value,
}

impl Event {
    pub fn new(seq: i64, ts: i64, data: &EventData) -> Event {
        // Splits the adjacently tagged value: `kind` is already known, `data` is the content.
        let content = match serde_json::to_value(data) {
            Ok(Value::Object(mut tagged)) => tagged.remove(DATA_KEY).unwrap_or_default(),
            _ => Value::Null,
        };

        Event {
            seq,
            ts,
            kind: data.kind().to_string(),
            subject: data.subject(),
            actor_client_id: None,
            data: content,
        }
    }
}

#[cfg(test)]
mod tests;
