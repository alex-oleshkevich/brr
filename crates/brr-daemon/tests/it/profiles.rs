#![allow(clippy::unwrap_used)]

use crate::common;

use std::{fs, path::PathBuf, sync::Arc};

use brr_core::{
    AdmissionMode, Build, BuildId, BuildSource, BuildState, Capability, ConflictReason, Display,
    Engine, Error, Instance, InstanceId, InstanceKind, InstanceState, Profile, ProfileId,
    ProfileKind, Template, now_ms,
};
use brr_daemon::{App, ops, services::profiles::DEFAULT_BACKUP_KEEP};
use brr_store::{build, instance, profile};
use futures::FutureExt;
use serde_json::{Value, json};

use common::{ctx, scratch_app};

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    let op = ops::find(name).unwrap();

    ops::dispatch(ctx(app), op, input).await
}

const QUOTA: u64 = 1_000_000;

async fn ok<T: serde::de::DeserializeOwned>(app: &Arc<App>, name: &str, input: Value) -> T {
    serde_json::from_value(call(app, name, input).await.unwrap()).unwrap()
}

fn conflict(result: Result<Value, Error>) -> ConflictReason {
    match result {
        Err(Error::Conflict { reason, .. }) => reason,
        other => panic!("expected Conflict, got {other:?}"),
    }
}

async fn published(app: &Arc<App>, kind: &str) -> usize {
    call(app, "events.list", json!({"kinds": kind}))
        .await
        .unwrap()["events"]
        .as_array()
        .unwrap()
        .len()
}

fn profile_row(name: &str, data_dir: Option<PathBuf>) -> Profile {
    Profile {
        id: ProfileId::new(),
        name: name.to_string(),
        kind: ProfileKind::Persistent,
        template_id: None,
        settings: Default::default(),
        pinned: false,
        notes: String::new(),
        tags: Vec::new(),
        data_dir,
        disk_quota_bytes: None,
        backup_schedule: None,
        backup_keep: DEFAULT_BACKUP_KEEP,
        created_at: now_ms(),
        updated_at: now_ms(),
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    }
}

#[tokio::test]
async fn builtins_seeded() {
    let (_dir, app) = scratch_app().await;

    let listed: Value = ok(&app, "template.list", json!({})).await;
    let templates: Vec<Template> = serde_json::from_value(listed["templates"].clone()).unwrap();
    let denied = call(
        &app,
        "template.delete",
        json!({"template": "chromium-headless"}),
    )
    .await;
    let renamed = call(
        &app,
        "template.update",
        json!({"template": "chromium-visible", "name": "mine"}),
    )
    .await;

    let names: Vec<&str> = templates.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "camoufox-headless",
            "camoufox-stealth",
            "chromium-headless",
            "chromium-visible"
        ]
    );
    assert!(templates.iter().all(|t| t.builtin));
    assert!(
        templates
            .iter()
            .all(|t| t.description.as_deref().is_some_and(|d| !d.is_empty()))
    );
    assert_eq!(conflict(denied), ConflictReason::Builtin);
    assert_eq!(conflict(renamed), ConflictReason::Builtin);
}

#[tokio::test]
async fn seeding_overwrites_builtin_settings_on_start() {
    let (_dir, app) = scratch_app().await;
    app.store
        .run(|c| {
            let mut found = brr_store::template::find(c, "chromium-visible")?;
            found.settings.display = Display::Headless;

            brr_store::template::update(c, &found)
        })
        .await
        .unwrap();

    brr_daemon::services::profiles::seed_templates(&app.store)
        .await
        .unwrap();

    let found: Template = ok(
        &app,
        "template.get",
        json!({"template": "chromium-visible"}),
    )
    .await;
    assert_eq!(found.settings.display, Display::Visible);
}

#[tokio::test]
async fn create_copies_template() {
    let (_dir, app) = scratch_app().await;

    let created: Profile = ok(
        &app,
        "profile.create",
        json!({"name": "work", "template": "chromium-headless", "tags": ["a"]}),
    )
    .await;
    let template: Template = ok(
        &app,
        "template.get",
        json!({"template": "chromium-headless"}),
    )
    .await;
    let again = call(
        &app,
        "profile.create",
        json!({"name": "work", "template": "chromium-headless"}),
    )
    .await;

    assert_eq!(created.settings, template.settings);
    assert_eq!(created.template_id, Some(template.id));
    assert_eq!(created.kind, ProfileKind::Persistent);
    assert_eq!(created.tags, ["a"]);
    let data_dir = created.data_dir.unwrap();
    assert_eq!(
        data_dir,
        app.paths.profiles_dir().join(created.id.to_string())
    );
    assert!(data_dir.is_dir());
    assert_eq!(published(&app, "profile.created").await, 1);
    assert_eq!(conflict(again), ConflictReason::NameTaken);
}

#[tokio::test]
async fn create_ephemeral_has_no_data_dir() {
    let (_dir, app) = scratch_app().await;

    let created: Profile = ok(
        &app,
        "profile.create",
        json!({"name": "tmp", "template": "chromium-visible", "kind": "ephemeral"}),
    )
    .await;

    assert_eq!(created.kind, ProfileKind::Ephemeral);
    assert_eq!(created.data_dir, None);
}

#[tokio::test]
async fn names_are_validated() {
    let (_dir, app) = scratch_app().await;

    let id_shaped = ProfileId::new().to_string();
    for name in ["", "a b", "a/b", &"x".repeat(65), &id_shaped] {
        let result = call(
            &app,
            "profile.create",
            json!({"name": name, "template": "chromium-headless"}),
        )
        .await;
        assert!(matches!(result, Err(Error::InvalidArgument { field, .. }) if field == "name"));
    }
    let bad_tag = call(
        &app,
        "profile.create",
        json!({"name": "ok", "template": "chromium-headless", "tags": ["a b"]}),
    )
    .await;
    assert!(matches!(bad_tag, Err(Error::InvalidArgument { field, .. }) if field == "tags"));
}

#[tokio::test]
async fn templates_do_not_touch_existing_profiles() {
    let (_dir, app) = scratch_app().await;
    let template: Template = ok(
        &app,
        "template.create",
        json!({"name": "mine", "settings": {"build": {"channel": "cft:stable"}, "display": "headless"}}),
    )
    .await;
    let created: Profile = ok(
        &app,
        "profile.create",
        json!({"name": "p", "template": template.id}),
    )
    .await;

    call(
        &app,
        "template.update",
        json!({"template": "mine", "settings": {"build": {"channel": "cft:beta"}, "display": "visible"}}),
    )
    .await
    .unwrap();

    let found: Profile = ok(&app, "profile.get", json!({"profile": "p"})).await;
    assert_eq!(found.settings, created.settings);
    assert_eq!(found.settings.display, Display::Headless);
}

#[tokio::test]
async fn save_from_profile_copies_settings() {
    let (_dir, app) = scratch_app().await;
    let created: Profile = ok(
        &app,
        "profile.create",
        json!({"name": "p", "template": "chromium-visible"}),
    )
    .await;

    let saved: Template = ok(
        &app,
        "template.save_from_profile",
        json!({"profile": created.id, "name": "saved"}),
    )
    .await;
    call(&app, "template.delete", json!({"template": "saved"}))
        .await
        .unwrap();

    assert!(!saved.builtin);
    assert_eq!(saved.settings, created.settings);
}

#[tokio::test]
async fn get_list_update_by_id_or_name() {
    let (_dir, app) = scratch_app().await;
    let created: Profile = ok(
        &app,
        "profile.create",
        json!({"name": "p", "template": "chromium-headless", "tags": ["x"]}),
    )
    .await;
    call(
        &app,
        "profile.create",
        json!({"name": "q", "template": "chromium-headless"}),
    )
    .await
    .unwrap();

    let by_id: Profile = ok(&app, "profile.get", json!({"profile": created.id})).await;
    let tagged: Value = ok(&app, "profile.list", json!({"tag": "x"})).await;
    let all: Value = ok(&app, "profile.list", json!({})).await;
    let updated: Profile = ok(
        &app,
        "profile.update",
        json!({"profile": "p", "name": "p2", "pinned": true, "notes": "n", "tags": ["y"]}),
    )
    .await;

    assert_eq!(by_id.name, "p");
    assert_eq!(tagged["profiles"].as_array().unwrap().len(), 1);
    // p, q and the built-in oneshot Profile.
    assert_eq!(all["profiles"].as_array().unwrap().len(), 3);
    assert_eq!(
        (updated.id, updated.name.as_str(), updated.pinned),
        (created.id, "p2", true)
    );
    assert_eq!(
        (updated.notes.as_str(), updated.tags),
        ("n", vec!["y".to_string()])
    );
    assert_eq!(published(&app, "profile.updated").await, 1);
}

#[tokio::test]
async fn capabilities_follow_the_build() {
    let (_dir, app) = scratch_app().await;
    let created: Profile = ok(
        &app,
        "profile.create",
        json!({"name": "p", "template": "chromium-headless"}),
    )
    .await;
    assert_eq!(created.capabilities, []);

    app.store
        .run(|c| {
            build::insert(
                c,
                &Build {
                    id: BuildId::new(),
                    source: BuildSource::System,
                    engine: Engine::Chromium,
                    name: "chrome".to_string(),
                    version: "1".to_string(),
                    channel: None,
                    path: PathBuf::from("/nonexistent"),
                    state: BuildState::Registered,
                    sha256: None,
                    size_bytes: 0,
                    capabilities: vec![Capability::Pdf],
                    installed_at: Some(now_ms()),
                    last_referenced_at: None,
                },
            )
        })
        .await
        .unwrap();

    let found: Profile = ok(&app, "profile.get", json!({"profile": "p"})).await;
    assert_eq!(found.capabilities, [Capability::Pdf]);
}

#[tokio::test]
async fn delete_removes_row_and_data_dir() {
    let (_dir, app) = scratch_app().await;
    let created: Profile = ok(
        &app,
        "profile.create",
        json!({"name": "p", "template": "chromium-headless"}),
    )
    .await;
    let data_dir = created.data_dir.clone().unwrap();

    let deleted: Profile = ok(&app, "profile.delete", json!({"profile": "p"})).await;

    assert_eq!(deleted.id, created.id);
    assert!(!data_dir.exists());
    assert!(matches!(
        call(&app, "profile.get", json!({"profile": "p"})).await,
        Err(Error::NotFound { .. })
    ));
    assert_eq!(published(&app, "profile.deleted").await, 1);
}

#[tokio::test]
async fn delete_guards() {
    let (_dir, app) = scratch_app().await;
    let outside = tempfile::tempdir().unwrap();
    let planted = profile_row("planted", Some(outside.path().to_path_buf()));
    let linked_target = tempfile::tempdir().unwrap();
    let mut linked = profile_row("linked", None);
    let link = app.paths.profiles_dir().join(linked.id.to_string());
    fs::create_dir_all(app.paths.profiles_dir()).unwrap();
    std::os::unix::fs::symlink(linked_target.path(), &link).unwrap();
    linked.data_dir = Some(link);
    let running = profile_row("running", None);
    let live = Instance {
        id: InstanceId::new(),
        kind: InstanceKind::Managed,
        profile_id: Some(running.id),
        build_id: None,
        state: InstanceState::Running,
        display: Display::Headless,
        priority: 0,
        admission: AdmissionMode::Queue,
        pid: None,
        log_path: None,
        cdp_url: None,
        data_dir: None,
        ephemeral: false,
        promote_to_name: None,
        restart_count: 0,
        last_activity_at: now_ms(),
        queued_at: now_ms(),
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
    };
    let rows = (planted.clone(), running.clone(), live, linked.clone());
    app.store
        .run(move |c| {
            profile::insert(c, &rows.0)?;
            profile::insert(c, &rows.1)?;
            profile::insert(c, &rows.3)?;

            instance::insert(c, &rows.2)
        })
        .await
        .unwrap();

    let outside_result = call(&app, "profile.delete", json!({"profile": planted.id})).await;
    let linked_result = call(&app, "profile.delete", json!({"profile": linked.id})).await;
    let running_result = call(&app, "profile.delete", json!({"profile": running.id})).await;

    assert!(matches!(outside_result, Err(Error::Internal { .. })));
    assert!(outside.path().is_dir());
    assert!(matches!(linked_result, Err(Error::Internal { .. })));
    assert!(linked_target.path().is_dir());
    assert_eq!(conflict(running_result), ConflictReason::ProfileRunning);
}

#[tokio::test]
async fn settings_validated() {
    let (_dir, app) = scratch_app().await;
    call(
        &app,
        "profile.create",
        json!({"name": "p", "template": "chromium-headless"}),
    )
    .await
    .unwrap();

    let result = call(
        &app,
        "profile.update",
        json!({"profile": "p", "settings": {
            "build": {"channel": "cft:stable"},
            "display": "headless",
            "args": ["--user-data-dir=/x"],
        }}),
    )
    .await;

    assert!(matches!(result, Err(Error::InvalidArgument { field, .. }) if field == "args"));
}

#[tokio::test]
async fn null_clears_the_disk_quota_and_absent_keeps_it() {
    let (_dir, app) = scratch_app().await;
    call(
        &app,
        "profile.create",
        json!({"name": "p", "template": "chromium-headless"}),
    )
    .await
    .unwrap();
    let quota = |profile: &Profile| profile.disk_quota_bytes;

    let set: Profile = ok(
        &app,
        "profile.update",
        json!({"profile": "p", "disk_quota_bytes": QUOTA}),
    )
    .await;
    let kept: Profile = ok(
        &app,
        "profile.update",
        json!({"profile": "p", "notes": "n"}),
    )
    .await;
    let cleared: Profile = ok(
        &app,
        "profile.update",
        json!({"profile": "p", "disk_quota_bytes": null}),
    )
    .await;

    assert_eq!(quota(&set), Some(QUOTA));
    assert_eq!(quota(&kept), Some(QUOTA));
    assert_eq!(quota(&cleared), None);
}

#[tokio::test]
async fn a_dropped_delete_still_removes_the_data_dir() {
    let (_dir, app) = scratch_app().await;
    let created: Profile = ok(
        &app,
        "profile.create",
        json!({"name": "p", "template": "chromium-headless"}),
    )
    .await;
    let data_dir = created.data_dir.clone().unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("Cookies"), "x").unwrap();

    // Polled once so the delete is under way, then dropped like a cancelled request.
    let op = ops::find("profile.delete").unwrap();
    let request = (op.handler)(ctx(&app), json!({"profile": "p"}));
    assert!(request.now_or_never().is_none());

    common::settled(async || !data_dir.exists()).await;
    assert!(matches!(
        call(&app, "profile.get", json!({"profile": "p"})).await,
        Err(Error::NotFound { .. })
    ));
}

#[tokio::test]
async fn events_name_the_client_that_caused_them() {
    let (_dir, app) = scratch_app().await;
    let caller = ctx(&app);
    let caller_id = caller.client.client_id.clone();

    ops::dispatch(
        caller,
        ops::find("profile.create").unwrap(),
        json!({"name": "work", "template": "chromium-headless"}),
    )
    .await
    .unwrap();
    app.events
        .publish(brr_core::EventData::ProfileDeleted {
            profile_id: ProfileId::new(),
            name: "daemon-side".to_string(),
        })
        .await
        .unwrap();

    let listed = call(
        &app,
        "events.list",
        json!({"kinds": "profile.created,profile.deleted"}),
    )
    .await
    .unwrap();
    let actor = |kind: &str| {
        listed["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["kind"] == kind)
            .unwrap()["actor_client_id"]
            .clone()
    };
    assert_eq!(actor("profile.created"), json!(caller_id));
    assert_eq!(actor("profile.deleted"), Value::Null);
}
