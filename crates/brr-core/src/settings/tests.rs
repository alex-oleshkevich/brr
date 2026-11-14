use std::{collections::BTreeMap, fs, net::SocketAddr, path::PathBuf};

use serde_json::json;
use tempfile::TempDir;

use super::{LimitMode, Source, load, patch};
use crate::{
    AdmissionMode, BackendKind, CaptchaPolicy, Error, LocalTime, Settings, Theme, UsdMicros,
};

const CONFIG_FILE: &str = "config.toml";
const COMMENT: &str = "# my favourite theme";
const THEMED_CONFIG: &str = "# my favourite theme\n[ui]\ntheme = \"light\"\n";

fn env(vars: &[(&str, &str)]) -> Vec<(String, String)> {
    vars.iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
}

fn config_in(dir: &TempDir, text: &str) -> PathBuf {
    let path = dir.path().join(CONFIG_FILE);
    fs::write(&path, text).unwrap();

    path
}

fn changes(leaf: &str, value: serde_json::Value) -> BTreeMap<String, serde_json::Value> {
    BTreeMap::from([(leaf.to_string(), value)])
}

#[test]
fn defaults_match_the_spec_table() {
    let settings = Settings::default();

    assert_eq!(
        settings.server.bind,
        "127.0.0.1:9898".parse::<SocketAddr>().unwrap()
    );
    assert_eq!(settings.server.socket, None);
    assert_eq!(settings.limits.mode, LimitMode::Static);
    assert_eq!(settings.limits.max_instances, 8);
    assert_eq!(settings.limits.max_instances_per_engine.chromium, 8);
    assert_eq!(settings.limits.max_instances_per_engine.camoufox, 4);
    assert!(settings.limits.max_instances_per_tag.is_empty());
    assert_eq!(settings.limits.max_tabs_per_instance, 20);
    assert_eq!(settings.limits.max_tabs_total, 100);
    assert_eq!(settings.adaptive.min_free_ram_mb, 1024);
    assert_eq!(settings.adaptive.max_load_per_core, 0.85);
    assert_eq!(settings.adaptive.reserve_mb.chromium_headless, 400);
    assert_eq!(settings.adaptive.reserve_mb.chromium_display, 600);
    assert_eq!(settings.adaptive.reserve_mb.camoufox, 600);
    assert_eq!(settings.adaptive.pressure_check_s, 5);
    assert_eq!(settings.admission.default, AdmissionMode::Queue);
    assert_eq!(settings.admission.queue_timeout_s, 60);
    assert_eq!(settings.admission.priority, 5);
    assert_eq!(settings.instance.start_timeout_s, 30);
    assert_eq!(settings.instance.stop_timeout_s, 10);
    assert!(!settings.instance.auto_restart);
    assert_eq!(settings.idle.tab_min, 15);
    assert_eq!(settings.idle.instance_min, 30);
    assert_eq!(settings.idle.grace_s, 60);
    assert_eq!(settings.lease.exclusive_wait_s, 60);
    assert_eq!(settings.vault.key_source, None);
    assert_eq!(settings.storage.public_endpoint, None);
    assert_eq!(settings.client.rest_idle_min, 10);
    assert_eq!(settings.client.mcp_http_idle_min, 10);
    assert_eq!(settings.oneshot.profile, "oneshot");
    assert_eq!(settings.oneshot.pool_size, 0);
    assert_eq!(settings.oneshot.linger_s, 60);
    assert_eq!(settings.results.inline_max_mb, 5);
    assert_eq!(settings.artifacts.url_ttl_s, 3600);
    assert_eq!(settings.tab.console_buffer, 1000);
    assert_eq!(settings.tab.network_buffer, 1000);
    assert_eq!(settings.login_check.interval_min, 60);
    assert_eq!(settings.proxy.health_interval_min, 60);
    assert_eq!(settings.captcha.daily_usd, UsdMicros(5_000_000));
    assert_eq!(settings.captcha.daily_solves, 200);
    assert_eq!(settings.captcha.human_wait_s, 300);
    assert_eq!(settings.captcha.default_policy, CaptchaPolicy::Auto);
    assert_eq!(settings.captcha.provider_url, "https://api.2captcha.com");
    assert_eq!(
        settings.content.adblock_lists,
        [
            "https://easylist.to/easylist/easylist.txt",
            "https://easylist.to/easylist/easyprivacy.txt"
        ]
    );
    assert_eq!(settings.builds.check_at, LocalTime { hour: 4, minute: 0 });
    assert_eq!(settings.builds.github_api, "https://api.github.com");
    assert_eq!(
        settings.builds.edge_repo,
        "https://packages.microsoft.com/repos/edge"
    );
    assert_eq!(
        settings.proxy.check_url,
        "https://www.cloudflare.com/cdn-cgi/trace"
    );
    assert_eq!(
        settings.extensions.cws_update_url,
        "https://clients2.google.com/service/update2/crx"
    );
    assert_eq!(
        settings.extensions.amo_api,
        "https://addons.mozilla.org/api/v5"
    );
    assert_eq!(settings.backup.keep, 7);
    assert_eq!(settings.retention.artifacts_max_age_days, 7);
    assert_eq!(settings.retention.artifacts_max_total_gb, 10);
    assert_eq!(settings.retention.download_max_mb, 2048);
    assert_eq!(settings.retention.free_disk_floor_gb, 5);
    assert_eq!(settings.retention.unused_days, 30);
    assert_eq!(settings.retention.events_days, 30);
    assert_eq!(settings.ui.theme, Theme::System);
}

#[test]
fn load_without_file_applies_env_over_defaults() {
    let dir = TempDir::new().unwrap();

    let (settings, sources) = load(
        &dir.path().join(CONFIG_FILE),
        &env(&[("BRR_LIMITS__MAX_INSTANCES", "4")]),
    )
    .unwrap();

    assert_eq!(settings.limits.max_instances, 4);
    assert_eq!(sources["limits.max_instances"], Source::Env);
    assert_eq!(sources["idle.tab_min"], Source::Default);
    assert_eq!(sources["server.socket"], Source::Default);
}

#[test]
fn load_marks_file_keys_and_env_wins_over_file() {
    let dir = TempDir::new().unwrap();
    let path = config_in(&dir, "[ui]\ntheme = \"light\"\n[idle]\ntab_min = 2\n");

    let (settings, sources) = load(&path, &env(&[("BRR_IDLE__TAB_MIN", "3")])).unwrap();

    assert_eq!(settings.ui.theme, Theme::Light);
    assert_eq!(settings.idle.tab_min, 3);
    assert_eq!(sources["ui.theme"], Source::File);
    assert_eq!(sources["idle.tab_min"], Source::Env);
}

#[test]
fn load_reads_bare_strings_and_ignores_unrelated_variables() {
    let dir = TempDir::new().unwrap();

    let (settings, _) = load(
        &dir.path().join(CONFIG_FILE),
        &env(&[
            ("BRR_UI__THEME", "dark"),
            ("BRR_TEST_CHROME", "/usr/bin/chrome"),
            ("BRR_INSTANCE_ID", "x"),
            ("BRR_IDLE__TAB_MIN", ""),
            ("HOME", "/h"),
        ]),
    )
    .unwrap();

    assert_eq!(settings.ui.theme, Theme::Dark);
    assert_eq!(settings.idle.tab_min, 15);
}

#[test]
fn load_reads_table_values_from_env() {
    let dir = TempDir::new().unwrap();

    let (settings, _) = load(
        &dir.path().join(CONFIG_FILE),
        &env(&[("BRR_LIMITS__MAX_INSTANCES_PER_ENGINE", "{ chromium = 2 }")]),
    )
    .unwrap();

    assert_eq!(settings.limits.max_instances_per_engine.chromium, 2);
    assert_eq!(settings.limits.max_instances_per_engine.camoufox, 4);
}

#[test]
fn load_rejects_unknown_env_key_and_section_naming_the_variable() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(CONFIG_FILE);

    for name in ["BRR_LIMITS__MAX_INSTANCEZ", "BRR_NOPE__KEY"] {
        let error = load(&path, &env(&[(name, "4")])).unwrap_err();

        assert!(matches!(error, Error::InvalidArgument { ref field, .. } if field == name));
    }
}

#[test]
fn load_rejects_a_wrong_typed_env_value_naming_the_variable() {
    let dir = TempDir::new().unwrap();

    let error = load(
        &dir.path().join(CONFIG_FILE),
        &env(&[("BRR_LIMITS__MAX_INSTANCES", "many")]),
    )
    .unwrap_err();

    assert!(
        matches!(error, Error::InvalidArgument { ref field, .. } if field == "BRR_LIMITS__MAX_INSTANCES")
    );
}

#[test]
fn load_rejects_unknown_file_keys() {
    let dir = TempDir::new().unwrap();
    let path = config_in(&dir, "[ui]\nthemee = \"dark\"\n");

    assert!(matches!(
        load(&path, &[]).unwrap_err(),
        Error::InvalidArgument { .. }
    ));
}

#[test]
fn patch_sets_value_and_keeps_comments() {
    let dir = TempDir::new().unwrap();
    let path = config_in(&dir, THEMED_CONFIG);

    patch(&path, &changes("ui.theme", json!("dark"))).unwrap();

    let (settings, _) = load(&path, &[]).unwrap();
    assert_eq!(settings.ui.theme, Theme::Dark);
    assert!(fs::read_to_string(&path).unwrap().starts_with(COMMENT));
}

#[test]
fn patch_creates_missing_file_and_section() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(CONFIG_FILE);

    patch(&path, &changes("limits.max_instances", json!(3))).unwrap();

    assert_eq!(load(&path, &[]).unwrap().0.limits.max_instances, 3);
}

#[test]
fn patch_with_null_removes_the_key() {
    let dir = TempDir::new().unwrap();
    let path = config_in(&dir, THEMED_CONFIG);

    patch(&path, &changes("ui.theme", json!(null))).unwrap();

    let (settings, sources) = load(&path, &[]).unwrap();
    assert_eq!(settings.ui.theme, Theme::System);
    assert_eq!(sources["ui.theme"], Source::Default);
}

#[test]
fn patch_rejects_bad_changes_and_leaves_the_file_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = config_in(&dir, THEMED_CONFIG);

    for (leaf, value) in [
        ("ui.theme", json!(3)),
        ("ui.themee", json!("dark")),
        ("nope.key", json!(1)),
        ("theme", json!("dark")),
    ] {
        let error = patch(&path, &changes(leaf, value)).unwrap_err();

        assert!(matches!(error, Error::InvalidArgument { ref field, .. } if field == leaf));
        assert_eq!(fs::read_to_string(&path).unwrap(), THEMED_CONFIG);
    }
}

#[test]
fn patch_reports_a_bad_number_without_toml_context() {
    let dir = TempDir::new().unwrap();
    let path = config_in(&dir, THEMED_CONFIG);

    let error = patch(&path, &changes("idle.tab_min", json!(-5))).unwrap_err();

    let Error::InvalidArgument { field, message } = error else {
        panic!("expected InvalidArgument");
    };
    assert_eq!(field, "idle.tab_min");
    assert!(
        !message.contains("TOML parse error") && !message.contains("line "),
        "{message}"
    );
}

#[test]
fn storage_backend() {
    let settings = Settings::default();

    assert_eq!(settings.storage.backend, BackendKind::Local);
    assert!(!settings.storage.path_style);
    assert_eq!(settings.storage.bucket, None);

    let dir = TempDir::new().unwrap();
    let (settings, _) = load(
        &dir.path().join(CONFIG_FILE),
        &env(&[("BRR_STORAGE__BACKEND", "s3")]),
    )
    .unwrap();

    assert_eq!(settings.storage.backend, BackendKind::S3);
}

#[test]
fn env_sets_captcha_default_policy() {
    let dir = TempDir::new().unwrap();

    let (settings, sources) = load(
        &dir.path().join(CONFIG_FILE),
        &env(&[("BRR_CAPTCHA__DEFAULT_POLICY", "human")]),
    )
    .unwrap();

    assert_eq!(settings.captcha.default_policy, CaptchaPolicy::Human);
    assert_eq!(sources["captcha.default_policy"], Source::Env);
    assert_eq!(sources["content.adblock_lists"], Source::Default);
}

const ZERO_MINIMUMS: [(&str, &str); 6] = [
    ("limits", "max_instances"),
    ("limits", "max_tabs_per_instance"),
    ("idle", "tab_min"),
    ("idle", "instance_min"),
    ("retention", "download_max_mb"),
    ("limits.max_instances_per_engine", "camoufox"),
];

#[test]
fn load_and_patch_reject_zero_minimums() {
    for (section, key) in ZERO_MINIMUMS {
        let dir = TempDir::new().unwrap();
        let leaf = format!("{section}.{key}");

        let from_file = load(&config_in(&dir, &format!("[{section}]\n{key} = 0\n")), &[]);
        assert!(
            matches!(from_file, Err(Error::InvalidArgument { ref field, .. }) if field.ends_with(key)),
            "{leaf}: {from_file:?}"
        );

        let env_name = format!(
            "BRR_{}__{}",
            section.replace('.', "_").to_uppercase(),
            key.to_uppercase()
        );
        if !section.contains('.') {
            let from_env = load(&dir.path().join("none.toml"), &env(&[(&env_name, "0")]));
            assert!(
                matches!(from_env, Err(Error::InvalidArgument { .. })),
                "{leaf}"
            );
        }

        let path = config_in(&dir, THEMED_CONFIG);
        let patched = patch(&path, &changes(&leaf, json!(0)));
        assert!(
            matches!(patched, Err(Error::InvalidArgument { .. })),
            "{leaf}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), THEMED_CONFIG);
    }
}

#[test]
fn patch_keeps_a_private_config_private() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let path = config_in(&dir, THEMED_CONFIG);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

    patch(&path, &changes("ui.theme", json!("dark"))).unwrap();

    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
