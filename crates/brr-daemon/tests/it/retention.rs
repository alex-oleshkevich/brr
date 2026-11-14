#![allow(clippy::unwrap_used)]

use crate::common;

use std::{fs, path::Path, sync::Arc};

use brr_core::{
    Artifact, ArtifactId, ArtifactKind, Event, EventData, InstanceId, InstanceState, Order, Page,
    ProfileId, ProfileKind, ProfileSettings, RetentionOverrides, model::BackendKind, now_ms,
};
use brr_daemon::{
    App, ops,
    services::{
        artifacts::{self, ArtifactOwner},
        retention::{self, SweepRule},
    },
};
use brr_store::{artifact, event, instance, profile, retention as store_retention};
use serde_json::{Value, json};

use common::{ctx, instance_row, profile_row, scratch_app};

const MS_PER_DAY: i64 = 86_400_000;
const MIB: u64 = 1024 * 1024;
const BIG_BYTES: u64 = 600 * MIB;
const OLD_EVENT_DAYS: i64 = 31;

async fn add_artifact(
    app: &Arc<App>,
    profile_id: Option<ProfileId>,
    age_days: i64,
    size_bytes: u64,
) -> ArtifactId {
    let id = ArtifactId::new();
    let location = format!("artifacts/{id}/f.bin");
    let path = app.paths.storage_dir().join(&location);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, [0]).unwrap();
    let row = Artifact {
        id,
        public_id: id.to_string(),
        kind: ArtifactKind::Pdf,
        profile_id,
        instance_id: None,
        tab_id: None,
        mime: "application/pdf".to_owned(),
        size_bytes,
        backend: BackendKind::Local,
        filename: "f.bin".to_owned(),
        location,
        sha256: String::new(),
        created_at: now_ms() - age_days * MS_PER_DAY,
    };
    app.store
        .run(move |c| artifact::insert(c, &row))
        .await
        .unwrap();

    id
}

async fn exists(app: &Arc<App>, artifact_id: ArtifactId) -> bool {
    app.store
        .run(move |c| artifact::get(c, &artifact_id))
        .await
        .is_ok()
}

fn object_exists(app: &Arc<App>, artifact_id: ArtifactId) -> bool {
    app.paths
        .storage_dir()
        .join(format!("artifacts/{artifact_id}/f.bin"))
        .exists()
}

async fn sweep_events(app: &Arc<App>) -> Vec<Value> {
    let query = event::Query {
        since: None,
        kinds: vec!["retention.sweep_completed".to_owned()],
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: Order::Asc,
        page: Page::new(Some(100), None),
    };
    let (events, _) = app
        .store
        .run(move |c| event::list(c, &query))
        .await
        .unwrap();

    events.into_iter().map(|event| event.data).collect()
}

fn ids(group: &retention::SweepGroup) -> Vec<String> {
    group.items.iter().map(|item| item.id.clone()).collect()
}

#[tokio::test]
async fn plan_equals_sweep() {
    let (_dir, app) = scratch_app().await;
    let now = now_ms();
    let settings = ProfileSettings {
        retention: RetentionOverrides {
            artifacts_max_age_days: None,
            artifacts_max_total_gb: Some(1),
        },
        ..ProfileSettings::default()
    };
    let mut found = profile_row(ProfileKind::Ephemeral, None, None);
    found.settings = settings;
    let profile_id = found.id;
    app.store
        .run(move |c| profile::insert(c, &found))
        .await
        .unwrap();
    let a = add_artifact(&app, None, 8, 10).await;
    let b = add_artifact(&app, Some(profile_id), 2, BIG_BYTES).await;
    let c = add_artifact(&app, Some(profile_id), 1, BIG_BYTES).await;

    let planned = retention::plan(&app, now).await.unwrap();

    assert_eq!(planned.len(), 2);
    assert_eq!(planned[0].rule, SweepRule::ArtifactsAge);
    assert_eq!(ids(&planned[0]), [a.to_string()]);
    assert_eq!(planned[1].rule, SweepRule::ArtifactsSize);
    assert_eq!(ids(&planned[1]), [b.to_string()]);

    let report = retention::sweep_at(&app, now, false).await.unwrap();

    assert_eq!(report.groups, planned);
    assert!(!exists(&app, a).await && !exists(&app, b).await);
    assert!(exists(&app, c).await);
    assert!(!object_exists(&app, a) && !object_exists(&app, b));
    assert!(object_exists(&app, c));
    let done = sweep_events(&app).await;
    assert_eq!(done[0]["deleted"]["artifacts"], 2);
    assert_eq!(done[0]["dry_run"], false);
}

fn make_dir(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::write(path.join("data"), [0; 16]).unwrap();
}

#[tokio::test]
async fn leftovers_and_events() {
    let (_dir, app) = scratch_app().await;
    let orphan = app
        .paths
        .ephemeral_dir()
        .join(InstanceId::new().to_string());
    make_dir(&orphan);
    let orphan_downloads = app
        .paths
        .downloads_dir()
        .join(InstanceId::new().to_string());
    make_dir(&orphan_downloads);
    let running_dir = {
        let id = InstanceId::new();
        app.paths.ephemeral_dir().join(id.to_string())
    };
    let mut running = instance_row(InstanceState::Running, Some(running_dir.clone()));
    // The row's id names the directory.
    let running_dir = app.paths.ephemeral_dir().join(running.id.to_string());
    running.data_dir = Some(running_dir.clone());
    make_dir(&running_dir);
    app.store
        .run(move |c| instance::insert(c, &running))
        .await
        .unwrap();

    let old = now_ms() - OLD_EVENT_DAYS * MS_PER_DAY;
    let cutoff = now_ms() - 30 * MS_PER_DAY;
    app.store
        .run(move |c| {
            for seq in [-1, -2] {
                let data = EventData::SettingsChanged { keys: Vec::new() };
                event::insert(c, &Event::new(seq, old, &data))?;
            }
            Ok(())
        })
        .await
        .unwrap();

    let report = retention::sweep(&app, false).await.unwrap();

    let rules: Vec<_> = report.groups.iter().map(|group| group.rule).collect();
    assert_eq!(rules, [SweepRule::EphemeralLeftovers, SweepRule::EventsAge]);
    assert_eq!(report.groups[0].items.len(), 2);
    assert_eq!(report.groups[1].items.len(), 1);
    assert!(
        report.groups[1].items[0]
            .label
            .starts_with("2 events older")
    );
    assert!(!orphan.exists() && !orphan_downloads.exists());
    assert!(running_dir.exists());
    let left = app
        .store
        .run(move |c| store_retention::events_before(c, cutoff))
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn zero_age_sweeps_all() {
    let (_dir, app) = scratch_app().await;
    let mut stored = Vec::new();
    for name in ["a.pdf", "b.pdf"] {
        let made = artifacts::store_bytes(
            &app,
            ArtifactKind::Pdf,
            ArtifactOwner::default(),
            name,
            "application/pdf",
            vec![1; 8].into(),
        )
        .await
        .unwrap();
        stored.push(made.id);
    }
    let call = |name: &'static str, input: Value| {
        let ctx = ctx(&app);

        async move {
            ops::dispatch(ctx, ops::find(name).unwrap(), input)
                .await
                .unwrap()
        }
    };

    let set = call("retention.set", json!({"artifacts_max_age_days": 0})).await;
    assert_eq!(set["artifacts_max_age_days"], 0);

    let preview = call("retention.sweep", json!({"dry_run": true})).await;
    assert_eq!(preview["groups"][0]["rule"], "artifacts_age");
    assert_eq!(preview["groups"][0]["items"].as_array().unwrap().len(), 2);
    for id in &stored {
        assert!(exists(&app, *id).await);
    }

    assert!(sweep_events(&app).await.is_empty());

    call("retention.sweep", json!({"dry_run": false})).await;
    for id in &stored {
        assert!(!exists(&app, *id).await);
    }
    let done = sweep_events(&app).await;
    assert_eq!(done.len(), 1);
    assert_eq!(done[0]["dry_run"], false);
    assert_eq!(done[0]["deleted"]["artifacts"], 2);
}
