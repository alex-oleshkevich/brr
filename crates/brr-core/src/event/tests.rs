use std::collections::BTreeMap;

use serde_json::json;
use test_case::test_case;

use crate::{
    AdmissionInputs, AdmissionOutcome, ArtifactId, ArtifactKind, BuildId, BuildSource,
    ClientEndReason, ClientId, ClientKind, CredentialId, Download, Event, EventData, ExtensionId,
    HumanReason, InstallPhase, InstanceId, InstanceState, InstanceTrigger, LeaseId, LoginStatus,
    ProfileId, ProxyId, ReleaseReason, SubjectType, SweepCounts, TabCloseReason, TabId, TabState,
    TabTrigger, UsdMicros,
};

const URL: &str = "https://example.com";

fn download() -> Download {
    Download {
        tab_id: TabId::new(),
        instance_id: InstanceId::new(),
        artifact_id: None,
        filename: "f.bin".to_string(),
        size: 1,
        reason: None,
    }
}

#[test_case(EventData::InstanceStateChanged { instance_id: InstanceId::new(), profile_id: None, from: None, to: InstanceState::Queued, trigger: InstanceTrigger::Launch, reason: None }, "instance.state_changed", false)]
#[test_case(EventData::InstanceQueued { instance_id: InstanceId::new(), queue_pos: 1, priority: 5 }, "instance.queued", false)]
#[test_case(EventData::InstanceReapingWarning { instance_id: InstanceId::new(), grace_ends_at: 1 }, "instance.reaping_warning", false)]
#[test_case(EventData::TabOpened { tab_id: TabId::new(), instance_id: InstanceId::new(), url: URL.into(), opened_by_brr: true }, "tab.opened", false)]
#[test_case(EventData::TabClosed { tab_id: TabId::new(), instance_id: InstanceId::new(), reason: TabCloseReason::Client }, "tab.closed", false)]
#[test_case(EventData::TabStateChanged { tab_id: TabId::new(), instance_id: InstanceId::new(), from: None, to: TabState::Opening, trigger: TabTrigger::Open }, "tab.state_changed", true)]
#[test_case(EventData::TabCrashed { tab_id: TabId::new(), instance_id: InstanceId::new() }, "tab.crashed", false)]
#[test_case(EventData::TabReapingWarning { tab_id: TabId::new(), instance_id: InstanceId::new(), grace_ends_at: 1 }, "tab.reaping_warning", false)]
#[test_case(EventData::TabAwaitingHuman { tab_id: TabId::new(), instance_id: InstanceId::new(), reason: HumanReason::Captcha, deadline: 1 }, "tab.awaiting_human", false)]
#[test_case(EventData::TabNavigated { tab_id: TabId::new(), instance_id: InstanceId::new(), url: URL.into(), title: "t".into() }, "tab.navigated", true)]
#[test_case(EventData::ClientCreated { client_id: ClientId::new(), kind: ClientKind::Cli, label: "l".into() }, "client.created", false)]
#[test_case(EventData::ClientEnded { client_id: ClientId::new(), reason: ClientEndReason::Ended }, "client.ended", false)]
#[test_case(EventData::LeaseAcquired { lease_id: LeaseId::new(), client_id: ClientId::new(), instance_id: InstanceId::new(), tab_id: None, exclusive: true }, "lease.acquired", false)]
#[test_case(EventData::LeaseReleased { lease_id: LeaseId::new(), client_id: ClientId::new(), instance_id: InstanceId::new(), tab_id: None, exclusive: true, reason: ReleaseReason::Released }, "lease.released", false)]
#[test_case(EventData::LeaseTransferred { lease_id: LeaseId::new(), instance_id: InstanceId::new(), from_client_id: ClientId::new(), to_client_id: ClientId::new() }, "lease.transferred", false)]
#[test_case(EventData::AdmissionDecided { instance_id: InstanceId::new(), outcome: AdmissionOutcome::Admitted, inputs: AdmissionInputs { free_ram_mb: 1, reserved_mb: 1, load_per_core: 0.5, counts: BTreeMap::new() } }, "admission.decided", false)]
#[test_case(EventData::MemoryPressure { free_ram_mb: 1, action: "a".into(), target_id: "t".into() }, "system.memory_pressure", false)]
#[test_case(EventData::BuildInstallProgress { build_id: BuildId::new(), bytes: 1, total: None, phase: InstallPhase::Downloading }, "build.install_progress", true)]
#[test_case(EventData::BuildInstalled { build_id: BuildId::new(), source: BuildSource::Cft, version: "1".into() }, "build.installed", false)]
#[test_case(EventData::BuildInstallFailed { build_id: BuildId::new(), source: BuildSource::Cft, version: "1".into(), error: "e".into() }, "build.install_failed", false)]
#[test_case(EventData::BuildRemoved { build_id: BuildId::new(), source: BuildSource::Cft, version: "1".into() }, "build.removed", false)]
#[test_case(EventData::BuildChannelMoved { source: BuildSource::Cft, channel: "stable".into(), from: None, to: "1".into() }, "build.channel_moved", false)]
#[test_case(EventData::ProfileCreated { profile_id: ProfileId::new(), name: "n".into() }, "profile.created", false)]
#[test_case(EventData::ProfileUpdated { profile_id: ProfileId::new(), name: "n".into() }, "profile.updated", false)]
#[test_case(EventData::ProfileDeleted { profile_id: ProfileId::new(), name: "n".into() }, "profile.deleted", false)]
#[test_case(EventData::ExtensionUpdated { extension_id: ExtensionId::new(), from: None, to: "1".into() }, "extension.updated", false)]
#[test_case(EventData::DownloadStarted(download()), "download.started", true)]
#[test_case(EventData::DownloadCompleted(download()), "download.completed", false)]
#[test_case(EventData::DownloadCancelled(download()), "download.cancelled", false)]
#[test_case(EventData::ArtifactCreated { artifact_id: ArtifactId::new(), kind: ArtifactKind::Pdf, size: 1 }, "artifact.created", false)]
#[test_case(EventData::ArtifactDeleted { artifact_id: ArtifactId::new(), kind: ArtifactKind::Pdf, size: 1, reason: None }, "artifact.deleted", false)]
#[test_case(EventData::SweepCompleted { dry_run: false, deleted: SweepCounts { artifacts: 1, builds: 0, extensions: 0, events: 0, backups: 0 }, bytes: 1 }, "retention.sweep_completed", false)]
#[test_case(EventData::BackupCompleted { profile_id: ProfileId::new(), backup_id: None }, "backup.completed", false)]
#[test_case(EventData::BackupFailed { profile_id: ProfileId::new(), error: "e".into() }, "backup.failed", false)]
#[test_case(EventData::BackupSkipped { profile_id: ProfileId::new() }, "backup.skipped", false)]
#[test_case(EventData::CaptchaDetected { tab_id: TabId::new(), instance_id: InstanceId::new(), site: "s".into(), captcha_type: "recaptcha".into() }, "captcha.detected", false)]
#[test_case(EventData::CaptchaSolved { tab_id: TabId::new(), instance_id: InstanceId::new(), site: "s".into(), captcha_type: "recaptcha".into(), cost_usd: UsdMicros(1), duration_ms: 1 }, "captcha.solved", false)]
#[test_case(EventData::CaptchaFailed { tab_id: TabId::new(), instance_id: InstanceId::new(), site: "s".into(), captcha_type: "recaptcha".into(), error: "e".into() }, "captcha.failed", false)]
#[test_case(EventData::CaptchaBudgetExhausted { spent_usd: UsdMicros(1), solves: 1 }, "captcha.budget_exhausted", false)]
#[test_case(EventData::LoginPerformed { tab_id: TabId::new(), instance_id: InstanceId::new(), credential_id: CredentialId::new(), ok: true, step: None }, "login.performed", false)]
#[test_case(EventData::LoginCheckResult { profile_id: ProfileId::new(), site: "s".into(), status: LoginStatus::Ok }, "login.check_result", false)]
#[test_case(EventData::LoginReloginSuspended { profile_id: ProfileId::new(), site: "s".into(), failures: 3 }, "login.relogin_suspended", false)]
#[test_case(EventData::ProxyHealth { proxy_id: ProxyId::new(), healthy: true, exit_ip: None, country: None, latency_ms: None }, "proxy.health", false)]
#[test_case(EventData::LimitsChanged { keys: vec![] }, "limits.changed", false)]
#[test_case(EventData::SettingsChanged { keys: vec![] }, "settings.changed", false)]
fn kind_matches_serde_tag_and_transient_flag(data: EventData, kind: &str, transient: bool) {
    let value = serde_json::to_value(&data).unwrap();

    assert_eq!(value["kind"], kind);
    assert_eq!(data.kind(), kind);
    assert_eq!(data.transient(), transient);
    assert_eq!(serde_json::from_value::<EventData>(value).unwrap(), data);
}

#[test]
fn new_splits_kind_and_data() {
    let instance_id = InstanceId::new();
    let data = EventData::InstanceQueued {
        instance_id,
        queue_pos: 2,
        priority: 5,
    };

    let event = Event::new(7, 1000, &data);

    assert_eq!((event.seq, event.ts), (7, 1000));
    assert_eq!(event.kind, "instance.queued");
    assert_eq!(event.subject.kind, SubjectType::Instance);
    assert_eq!(event.subject.id, instance_id.to_string());
    assert_eq!(
        event.data,
        json!({ "instance_id": instance_id.to_string(), "queue_pos": 2, "priority": 5 })
    );
}

#[test]
fn subject_of_channel_move_names_source_and_channel() {
    let data = EventData::BuildChannelMoved {
        source: BuildSource::Cft,
        channel: "stable".to_string(),
        from: None,
        to: "1".to_string(),
    };

    let subject = data.subject();

    assert_eq!(subject.kind, SubjectType::Build);
    assert_eq!(subject.id, "cft:stable");
}

#[test]
fn subject_of_download_is_its_tab() {
    let download = download();
    let tab_id = download.tab_id;

    assert_eq!(
        EventData::DownloadCompleted(download).subject().id,
        tab_id.to_string()
    );
}

#[test]
fn subject_of_settings_is_system() {
    let subject = EventData::SettingsChanged { keys: vec![] }.subject();

    assert_eq!(
        (subject.kind, subject.id.as_str()),
        (SubjectType::System, "settings")
    );
}
