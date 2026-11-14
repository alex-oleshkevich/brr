#![allow(clippy::unwrap_used)]

use crate::common;

use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};

use brr_core::{ConflictReason, Error, EventData, KeySource, ProfileId, settings};
use brr_daemon::{App, ops, services::backups::PASSPHRASE_SECRET};
use serde_json::{Value, json};
use test_case::test_case;
use tokio::time::Instant;

use common::{VERSION, boot, ctx, paths, scratch_app};

const OPS: [&str; 5] = [
    "events.list",
    "events.watch",
    "settings.get",
    "settings.set",
    "system.info",
];
const WATCH_TIMEOUT_MS: u64 = 2000;
const THEME_KEY: &str = "ui.theme";
const THEME_ENV: &str = "BRR_UI__THEME";
const PRIVATE_DIR_MODE: u32 = 0o700;
const OPEN_DIR_MODE: u32 = 0o755;

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    let op = ops::find(name).unwrap();

    ops::dispatch(ctx(app), op, input).await
}

fn profile_created() -> EventData {
    EventData::ProfileCreated {
        profile_id: ProfileId::new(),
        name: "work".to_string(),
    }
}

#[test]
fn registry_lookup() {
    let names: Vec<&str> = ops::all().iter().map(|op| op.spec.name).collect();

    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    for name in OPS {
        assert!(ops::find(name).is_some(), "{name} is registered");
    }
    assert_eq!(
        ops::find_mcp("events_watch").unwrap().spec.name,
        "events.watch"
    );
    assert!(ops::find("no.such").is_none());
}

#[tokio::test(start_paused = true)]
async fn events_watch_returns_the_next_event() {
    let (_dir, app) = scratch_app().await;

    let watching = {
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            call(
                &app,
                "events.watch",
                json!({"timeout_ms": WATCH_TIMEOUT_MS}),
            )
            .await
        })
    };
    // One yield runs the watcher up to its first real await, which is after it subscribed.
    tokio::task::yield_now().await;
    let published = app.events.publish(profile_created()).await.unwrap();

    let result = watching.await.unwrap().unwrap();

    assert_eq!(result["events"][0]["seq"], published.event.seq);
    assert_eq!(result["events"][0]["kind"], "profile.created");
    assert_eq!(result["next_since"], published.event.seq);
}

#[tokio::test(start_paused = true)]
async fn events_watch_times_out_empty() {
    let (_dir, app) = scratch_app().await;
    let started = Instant::now();

    let result = call(
        &app,
        "events.watch",
        json!({"timeout_ms": WATCH_TIMEOUT_MS}),
    )
    .await
    .unwrap();

    assert_eq!(result["events"], json!([]));
    assert_eq!(result["next_since"], 0);
    assert!(started.elapsed() >= Duration::from_millis(WATCH_TIMEOUT_MS));
}

#[tokio::test(start_paused = true)]
async fn events_watch_skips_other_kinds() {
    let (_dir, app) = scratch_app().await;

    let watching = {
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            call(
                &app,
                "events.watch",
                json!({"kinds": "profile.deleted", "timeout_ms": WATCH_TIMEOUT_MS}),
            )
            .await
        })
    };
    tokio::task::yield_now().await;
    app.events.publish(profile_created()).await.unwrap();

    let result = watching.await.unwrap().unwrap();

    assert_eq!(result["events"], json!([]));
}

#[tokio::test]
async fn settings_set() {
    let (dir, app) = scratch_app().await;

    let set = call(
        &app,
        "settings.set",
        json!({THEME_KEY: "dark", "server.bind": "127.0.0.1:9999"}),
    )
    .await
    .unwrap();
    let got = call(&app, "settings.get", json!({})).await.unwrap();

    assert_eq!(set["restart_required"], json!(["server.bind"]));
    assert_eq!(got["settings"]["ui"]["theme"], "dark");
    assert_eq!(got["sources"][THEME_KEY], "file");
    assert_eq!(got["sources"]["limits.mode"], "default");
    let config = std::fs::read_to_string(paths(dir.path()).config_file()).unwrap();
    assert!(config.contains("theme = \"dark\""));

    let changed = call(&app, "events.list", json!({"kinds": "settings.changed"}))
        .await
        .unwrap();
    assert_eq!(
        changed["events"][0]["data"]["keys"],
        json!(["server.bind", THEME_KEY])
    );
}

#[test_case("vault.key_source", json!("keyring"); "vault source is fixed")]
#[test_case(THEME_KEY, json!("no-such-theme"); "invalid value")]
#[test_case("ui.no_such_key", json!(1); "unknown key")]
#[test_case("idle.tab_min", json!(0); "zero tab minutes")]
#[test_case("idle.instance_min", json!(0); "zero instance minutes")]
#[tokio::test]
async fn settings_set_rejects(key: &str, value: Value) {
    let (_dir, app) = scratch_app().await;

    let error = call(&app, "settings.set", json!({key: value}))
        .await
        .unwrap_err();

    assert!(matches!(error, Error::InvalidArgument { .. }), "{error:?}");
}

#[tokio::test]
async fn a_rejected_patch_does_not_replace_the_backup_passphrase() {
    let (_dir, app) = scratch_app().await;
    let status = async || {
        let got = call(&app, "settings.get", json!({})).await.unwrap();

        got["secrets"][PASSPHRASE_SECRET].clone()
    };
    let before = status().await;

    let rejected = call(
        &app,
        "settings.set",
        json!({PASSPHRASE_SECRET: "new-secret", "idle.tab_min": 0}),
    )
    .await;
    assert!(matches!(rejected, Err(Error::InvalidArgument { .. })));
    assert_eq!(status().await, before);

    call(
        &app,
        "settings.set",
        json!({PASSPHRASE_SECRET: "new-secret"}),
    )
    .await
    .unwrap();
    assert_ne!(status().await, before);
}

#[tokio::test]
async fn settings_set_rejects_env_keys() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[(THEME_ENV, "light")]))
        .await
        .unwrap();

    let got = call(&app, "settings.get", json!({})).await.unwrap();
    let error = call(&app, "settings.set", json!({THEME_KEY: "dark"}))
        .await
        .unwrap_err();

    assert_eq!(got["sources"][THEME_KEY], "env");
    assert!(
        matches!(&error, Error::InvalidArgument { field, message }
            if field == THEME_KEY && message.contains(THEME_ENV)),
        "{error:?}"
    );
}

#[tokio::test]
async fn system_info() {
    let (_dir, app) = scratch_app().await;

    let info = call(&app, "system.info", json!({})).await.unwrap();

    assert_eq!(info["version"], VERSION);
    let expected = paths(_dir.path());
    assert_eq!(
        info["data_dir"],
        expected.data_dir.to_string_lossy().into_owned()
    );
    assert_eq!(
        info["config_path"],
        expected.config_file().to_string_lossy().into_owned()
    );
    assert!(info["logical_cpus"].as_u64().unwrap() >= 1);
    assert!(info["mem_available_mb"].as_u64() <= info["mem_total_mb"].as_u64());
}

#[tokio::test]
async fn startup_guards() {
    let (dir, app) = scratch_app().await;
    let key_file = paths(dir.path()).vault_key_file();

    let second = App::start(boot(dir.path(), &[])).await.err().unwrap();
    assert_eq!(
        second,
        Error::Conflict {
            reason: ConflictReason::DaemonRunning,
            detail: None
        }
    );

    // Another runtime dir (a shell without XDG_RUNTIME_DIR) must not allow a second daemon on the data.
    let mut elsewhere = boot(dir.path(), &[]);
    elsewhere.paths.runtime_dir = dir.path().join("other-run").join("brr");
    let third = App::start(elsewhere).await.err().unwrap();
    assert!(
        matches!(
            third,
            Error::Conflict {
                reason: ConflictReason::DaemonRunning,
                ..
            }
        ),
        "{third:?}"
    );

    drop(app);
    std::fs::remove_file(&key_file).unwrap();

    let restarted = App::start(boot(dir.path(), &[])).await.err().unwrap();
    assert!(
        matches!(restarted, Error::DependencyMissing { .. }),
        "{restarted:?}"
    );
    assert!(!key_file.exists(), "a new key must never be created");
}

#[tokio::test]
async fn runtime_dir_must_be_private() {
    let dir = tempfile::tempdir().unwrap();
    let runtime_dir = paths(dir.path()).runtime_dir;
    std::fs::create_dir_all(&runtime_dir).unwrap();
    std::fs::set_permissions(&runtime_dir, std::fs::Permissions::from_mode(OPEN_DIR_MODE)).unwrap();

    let error = App::start(boot(dir.path(), &[])).await.err().unwrap();

    assert!(matches!(error, Error::Internal { .. }), "{error:?}");
    std::fs::set_permissions(
        &runtime_dir,
        std::fs::Permissions::from_mode(PRIVATE_DIR_MODE),
    )
    .unwrap();
    assert!(App::start(boot(dir.path(), &[])).await.is_ok());
}

#[tokio::test]
async fn runtime_dir_symlink_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let runtime_dir = paths(dir.path()).runtime_dir;
    let target = dir.path().join("elsewhere");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(PRIVATE_DIR_MODE)).unwrap();
    std::fs::create_dir_all(runtime_dir.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &runtime_dir).unwrap();

    let error = App::start(boot(dir.path(), &[])).await.err().unwrap();

    assert!(matches!(error, Error::Internal { .. }), "{error:?}");
}

#[tokio::test]
async fn fresh_vault_pins_the_default_key_source() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = boot(dir.path(), &[]);
    std::fs::write(first.paths.config_file(), "").unwrap();
    (first.settings, first.sources) = settings::load(&first.paths.config_file(), &[]).unwrap();
    assert_eq!(first.settings.vault.key_source, None);

    let app = App::start(first).await.unwrap();
    let config = std::fs::read_to_string(paths(dir.path()).config_file()).unwrap();
    assert!(config.contains("key_source = \"file\""), "{config}");
    assert_eq!(app.settings().vault.key_source, Some(KeySource::File));
    drop(app);

    let mut second = boot(dir.path(), &[]);
    (second.settings, second.sources) = settings::load(&second.paths.config_file(), &[]).unwrap();
    assert!(App::start(second).await.is_ok());
}
