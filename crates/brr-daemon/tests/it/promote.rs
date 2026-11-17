#![allow(clippy::unwrap_used)]

use crate::common;

use std::{fs, sync::Arc};

use brr_core::{
    AdmissionMode, ClientId, ClientKind, ConflictReason, EndReason, Error, IdleOverrides,
    InstanceId, InstanceState, Profile, ProfileKind,
};
use brr_daemon::{App, CallClient, ops, services::instances::LaunchRequest};
use brr_store::{Store, instance, profile};
use serde_json::{Value, json};

use common::{
    KillOnDrop, add_profile, boot, chrome_app, ctx, instance_row, paths, row, scratch_app,
};

const MARKER_FILE: &str = "kept-marker.txt";
const MARKER: &str = "survives the promotion";
const PROFILE_CREATED: &str = "profile.created";

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    let mut context = ctx(app);
    context.client = CallClient {
        client_id: ClientId::new(),
        kind: ClientKind::Cli,
        label: "promote".to_owned(),
        implicit: true,
    };

    ops::dispatch(context, ops::find(name).unwrap(), input).await
}

async fn find_profile(app: &Arc<App>, name: &str) -> Result<Profile, Error> {
    let name = name.to_owned();

    app.store.run(move |c| profile::find(c, &name)).await
}

#[tokio::test]
async fn promote_moves_data() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    let source = add_profile(&app, build_id, "scratch", false, IdleOverrides::default()).await;
    let (running, _) = app
        .instances
        .launch(
            &app,
            LaunchRequest {
                profile: source.clone(),
                display: None,
                priority: 5,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);
    let ephemeral_dir = running.data_dir.clone().unwrap();
    fs::write(ephemeral_dir.join(MARKER_FILE), MARKER).unwrap();

    let promoted = call(
        &app,
        "instance.promote",
        json!({ "instance_id": running.id.to_string(), "name": "kept" }),
    )
    .await
    .unwrap();
    assert_eq!(promoted["promote_to_name"], "kept");

    let stopped = app
        .instances
        .stop(&app, &running.id, EndReason::Stop)
        .await
        .unwrap();
    assert_eq!(stopped.state, InstanceState::Stopped);

    let kept = find_profile(&app, "kept").await.unwrap();
    assert_eq!(kept.kind, ProfileKind::Persistent);
    assert_eq!(kept.settings, source.settings);
    let kept_dir = kept.data_dir.unwrap();
    assert_eq!(
        fs::read_to_string(kept_dir.join(MARKER_FILE)).unwrap(),
        MARKER
    );
    assert!(!ephemeral_dir.exists());

    let all = app
        .store
        .run(|c| {
            brr_store::event::list(
                c,
                &brr_store::event::Query {
                    since: None,
                    kinds: vec![PROFILE_CREATED.to_owned()],
                    subject: None,
                    instance_id: None,
                    from_ts: None,
                    to_ts: None,
                    order: brr_core::Order::Asc,
                    page: brr_core::Page::new(Some(100), None),
                },
            )
        })
        .await
        .unwrap()
        .0;
    assert!(all.iter().any(|event| event.data["name"] == "kept"));
}

#[tokio::test]
async fn promote_refusals() {
    let (dir, app) = scratch_app().await;
    let persistent = instance_row(InstanceState::Running, None);
    let mut stopped = instance_row(InstanceState::Stopped, Some(dir.path().join("gone")));
    stopped.ended_at = Some(1);
    let ids = (persistent.id, stopped.id);
    app.store
        .run(move |c| {
            instance::insert(c, &persistent)?;
            instance::insert(c, &stopped)
        })
        .await
        .unwrap();
    let taken = add_taken_profile(&app).await;

    let refused = call(
        &app,
        "instance.promote",
        json!({ "instance_id": ids.0.to_string(), "name": "fresh" }),
    )
    .await;
    assert!(matches!(
        refused,
        Err(Error::InvalidArgument { field, .. }) if field == "instance_id"
    ));

    let refused = call(
        &app,
        "instance.promote",
        json!({ "instance_id": ids.1.to_string(), "name": "fresh" }),
    )
    .await;
    assert_eq!(
        refused.unwrap_err(),
        Error::InvalidTransition {
            from: "stopped".to_owned(),
            to: "promoted".to_owned()
        }
    );

    let refused = call(
        &app,
        "instance.promote",
        json!({ "instance_id": ids.0.to_string(), "name": taken.name }),
    )
    .await;
    assert!(matches!(
        refused,
        Err(Error::Conflict {
            reason: ConflictReason::NameTaken,
            ..
        })
    ));
}

async fn add_taken_profile(app: &Arc<App>) -> Profile {
    let created = source_profile("taken");
    let row = created.clone();
    app.store
        .run(move |c| profile::insert(c, &row))
        .await
        .unwrap();

    created
}

fn source_profile(name: &str) -> Profile {
    let now = brr_core::now_ms();

    Profile {
        id: brr_core::ProfileId::new(),
        name: name.to_owned(),
        kind: ProfileKind::Ephemeral,
        template_id: None,
        template_name: None,
        settings: brr_core::ProfileSettings::default(),
        pinned: false,
        notes: String::new(),
        tags: vec!["scrape".to_owned()],
        data_dir: None,
        disk_quota_bytes: None,
        backup_schedule: None,
        backup_keep: 0,
        created_at: now,
        updated_at: now,
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    }
}

#[tokio::test]
async fn reconcile_promotes() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    fs::create_dir_all(&paths.data_dir).unwrap();

    let ephemeral_data = paths.ephemeral_dir().join("late-instance");
    fs::create_dir_all(&ephemeral_data).unwrap();
    fs::write(ephemeral_data.join(MARKER_FILE), MARKER).unwrap();
    let leftover = paths.ephemeral_dir().join("leftover");
    fs::create_dir_all(&leftover).unwrap();

    let source = source_profile("source");
    let mut stale = instance_row(InstanceState::Running, Some(ephemeral_data.clone()));
    stale.profile_id = Some(source.id);
    stale.promote_to_name = Some("late".to_owned());
    let stale_id: InstanceId = stale.id;
    let store = Store::open(&paths.db_file()).unwrap();
    store
        .run(move |c| {
            profile::insert(c, &source)?;
            instance::insert(c, &stale)
        })
        .await
        .unwrap();
    drop(store);

    let app = App::start(boot(dir.path(), &[])).await.unwrap();

    assert_eq!(row(&app, stale_id).await.state, InstanceState::Stopped);
    let late = find_profile(&app, "late").await.unwrap();
    assert_eq!(late.tags, ["scrape"]);
    assert_eq!(
        fs::read_to_string(late.data_dir.unwrap().join(MARKER_FILE)).unwrap(),
        MARKER
    );
    assert!(!ephemeral_data.exists());
    assert!(!leftover.exists());
}

const UNPROMOTED_DIR: &str = "unpromoted";

// A stopped row whose data is still in the ephemeral dir: the daemon died between the stop and
// the move. The restart finishes the promotion instead of sweeping the data.
#[tokio::test]
async fn reconcile_finishes_an_interrupted_promotion() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    fs::create_dir_all(&paths.data_dir).unwrap();

    let source = source_profile("source");
    let mut stopped = instance_row(InstanceState::Stopped, None);
    let data = paths.ephemeral_dir().join(stopped.id.to_string());
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join(MARKER_FILE), MARKER).unwrap();
    stopped.data_dir = Some(data.clone());
    stopped.ephemeral = true;
    stopped.profile_id = Some(source.id);
    stopped.promote_to_name = Some("rescued".to_owned());
    stopped.ended_at = Some(1);
    let id = stopped.id;
    let store = Store::open(&paths.db_file()).unwrap();
    store
        .run(move |c| {
            profile::insert(c, &source)?;
            instance::insert(c, &stopped)
        })
        .await
        .unwrap();
    drop(store);

    let app = App::start(boot(dir.path(), &[])).await.unwrap();

    let rescued = find_profile(&app, "rescued").await.unwrap();
    assert_eq!(
        fs::read_to_string(rescued.data_dir.unwrap().join(MARKER_FILE)).unwrap(),
        MARKER
    );
    assert!(!data.exists());
    assert_eq!(row(&app, id).await.state, InstanceState::Stopped);
}

// The promotion cannot finish (its source Profile is gone): the data must survive the restart.
#[tokio::test]
async fn failed_promotion_keeps_the_data_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let paths = paths(dir.path());
    fs::create_dir_all(&paths.data_dir).unwrap();

    // No source Profile on the row: the promotion cannot be carried out.
    let mut stopped = instance_row(InstanceState::Stopped, None);
    let data = paths.ephemeral_dir().join(stopped.id.to_string());
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join(MARKER_FILE), MARKER).unwrap();
    stopped.data_dir = Some(data.clone());
    stopped.ephemeral = true;
    stopped.promote_to_name = Some("never".to_owned());
    stopped.ended_at = Some(1);
    let id = stopped.id;
    let store = Store::open(&paths.db_file()).unwrap();
    store
        .run(move |c| instance::insert(c, &stopped))
        .await
        .unwrap();
    drop(store);

    let first = App::start(boot(dir.path(), &[])).await.unwrap();
    drop(first);
    let second = App::start(boot(dir.path(), &[])).await.unwrap();

    let aside = paths.data_dir.join(UNPROMOTED_DIR).join(id.to_string());
    assert_eq!(fs::read_to_string(aside.join(MARKER_FILE)).unwrap(), MARKER);
    assert!(find_profile(&second, "never").await.is_err());
}
