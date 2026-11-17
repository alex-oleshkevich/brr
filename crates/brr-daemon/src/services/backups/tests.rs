use std::{ffi::OsString, fs, sync::Arc, time::Duration};

use brr_core::{
    BackupSchedule, ConflictReason, DisplayEnv, Engine, Error, Paths, Profile, ProfileId,
    ProfileKind, ProfileSettings, Secret, now_ms, settings,
};
use brr_store::{backup, event, profile};
use tempfile::TempDir;
use test_case::test_case;

use super::{
    PASSPHRASE_SECRET, SLEEP_CHUNK, after_sleep, backup_one, due_at, scheduled, suffixed_name,
    wait_for,
};
use crate::{
    App, Boot,
    services::{
        cookies::{Target, apply, ensure_not_exporting},
        profiles::{DEFAULT_BACKUP_KEEP, MAX_NAME_LEN, validate_name},
    },
};

const CONFIG_WITH_FILE_VAULT: &str = "[vault]\nkey_source = \"file\"\n";
const COOKIES: &[u8] = b"cookies";
const SCHEDULE: &str = "daily@03:00";
const AFTER_MS: i64 = 1_000_000_000_000;

async fn scratch_app() -> (TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths {
        config_dir: dir.path().join("config"),
        data_dir: dir.path().join("data"),
        state_dir: dir.path().join("state"),
        runtime_dir: dir.path().join("run").join("brr"),
    };
    fs::create_dir_all(&paths.config_dir).unwrap();
    fs::write(paths.config_file(), CONFIG_WITH_FILE_VAULT).unwrap();
    let (settings, sources) = settings::load(&paths.config_file(), &[]).unwrap();

    let app = App::start(Boot {
        paths,
        settings,
        sources,
        env: Vec::new(),
        display_env: DisplayEnv::default(),
        path_env: OsString::new(),
        cft_base_url: String::new(),
        version: "0.0.0-test",
    })
    .await
    .unwrap();

    (dir, app)
}

fn scheduled_profile(app: &App, schedule: Option<&str>) -> Profile {
    let name = if schedule.is_some() {
        "scheduled"
    } else {
        "unscheduled"
    };
    let data_dir = app.paths.profiles_dir().join(name);
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("Cookies"), COOKIES).unwrap();
    let now = now_ms();

    Profile {
        id: ProfileId::new(),
        name: name.to_string(),
        kind: ProfileKind::Persistent,
        template_id: None,
        template_name: None,
        settings: ProfileSettings::default(),
        pinned: false,
        notes: String::new(),
        tags: Vec::new(),
        data_dir: Some(data_dir),
        disk_quota_bytes: None,
        backup_schedule: schedule.map(str::to_owned),
        backup_keep: DEFAULT_BACKUP_KEEP,
        created_at: now,
        updated_at: now,
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    }
}

async fn backup_count(app: &App, profile_id: ProfileId) -> usize {
    app.store
        .run(move |c| backup::for_profile(c, &profile_id))
        .await
        .unwrap()
        .len()
}

async fn completed(app: &App) -> usize {
    let query = event::Query {
        since: None,
        kinds: vec!["backup.completed".to_string()],
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: brr_core::Order::Asc,
        page: brr_core::Page::new(None, None),
    };

    app.store
        .run(move |c| event::list(c, &query))
        .await
        .unwrap()
        .0
        .len()
}

async fn insert(app: &App, found: &Profile) {
    let found = found.clone();

    app.store
        .run(move |c| profile::insert(c, &found))
        .await
        .unwrap();
}

#[tokio::test]
async fn scheduled_lists_only_profiles_with_a_schedule() {
    let (_dir, app) = scratch_app().await;
    let with = scheduled_profile(&app, Some(SCHEDULE));
    let without = scheduled_profile(&app, None);
    insert(&app, &with).await;
    insert(&app, &without).await;
    let expected = SCHEDULE
        .parse::<BackupSchedule>()
        .unwrap()
        .next_after(AFTER_MS);

    assert_eq!(scheduled(&app, AFTER_MS).await, [(with.id, expected)]);
}

#[tokio::test]
async fn a_due_backup_is_stored_and_reported() {
    let (_dir, app) = scratch_app().await;
    app.secrets
        .set(&app.store, PASSPHRASE_SECRET, &Secret::new("pp"))
        .await
        .unwrap();
    let found = scheduled_profile(&app, Some(SCHEDULE));
    insert(&app, &found).await;

    backup_one(&app, found.id).await;
    assert_eq!(backup_count(&app, found.id).await, 1);
    assert_eq!(completed(&app).await, 1);

    backup_one(&app, found.id).await;
    assert_eq!(backup_count(&app, found.id).await, 2);
    assert_eq!(completed(&app).await, 2);
}

#[test_case(10_000, 0, Duration::from_millis(10_000); "short wait")]
#[test_case(i64::MAX / 2, 0, SLEEP_CHUNK; "capped at a chunk")]
#[test_case(0, 10_000, Duration::ZERO; "already past")]
fn wait_for_caps_the_sleep(next: i64, cursor: i64, expected: Duration) {
    assert_eq!(wait_for(next, cursor), expected);
}

#[test_case(1_000, 500, 100, 1_500; "wall clock behind: the cursor moves by the sleep")]
#[test_case(1_000, 500, 9_000, 9_000; "wall clock ahead: the cursor follows it")]
fn after_sleep_never_trails_the_wall_clock(cursor: i64, waited_ms: u64, wall: i64, expected: i64) {
    assert_eq!(
        after_sleep(cursor, Duration::from_millis(waited_ms), wall),
        expected
    );
}

#[test]
fn due_at_picks_only_the_earliest_profiles() {
    let (first, second, later) = (ProfileId::new(), ProfileId::new(), ProfileId::new());

    let due = due_at(vec![(first, 10), (later, 20), (second, 10)], 10);

    assert_eq!(due, [first, second]);
}

#[test]
fn an_imported_name_with_a_suffix_stays_within_the_limit() {
    let base = "a".repeat(MAX_NAME_LEN);

    let name = suffixed_name(&base, 12);

    assert_eq!(name.len(), MAX_NAME_LEN);
    assert!(name.ends_with("-12"));
    validate_name(&name, "name").unwrap();
    assert_eq!(suffixed_name("alpha", 1), "alpha-1");
}

#[test]
fn export_guard_counts_overlapping_exports() {
    let exports = super::Exports::default();
    let profile_id = brr_core::ProfileId::new();

    let first = exports.begin(profile_id);
    let second = exports.begin(profile_id);
    drop(first);
    assert!(exports.is_exporting(&profile_id));
    drop(second);

    assert!(!exports.is_exporting(&profile_id));
}

#[tokio::test]
async fn offline_cookie_writes_wait_for_an_export() {
    let (_dir, app) = scratch_app().await;
    let found = scheduled_profile(&app, None);
    let target = Target::Files(found.data_dir.clone().unwrap(), Engine::Chromium);
    let exporting = app.exports.begin(found.id);

    let refused = apply(&app, &found, Vec::new(), &target).await;

    assert!(matches!(
        refused,
        Err(Error::Conflict {
            reason: ConflictReason::ProfileRunning,
            ..
        })
    ));
    drop(exporting);
    assert!(ensure_not_exporting(&app, &found.id).is_ok());
}
