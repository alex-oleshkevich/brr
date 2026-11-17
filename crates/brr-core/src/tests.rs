use std::collections::BTreeMap;

use schemars::schema_for;
use serde_json::json;
use test_case::test_case;

use crate::{
    BuildChoice, BuildSource, Channel, ClientId, ConflictReason, Credential, Error, InstanceId,
    Page, Profile, ProfileId, ProfileKind, ProfileSettings, Secret, TabId, Toolset, UsdMicros,
    site_origin,
};

const NOT_A_ULID: &str = "not-a-ulid";
const SECRET_VALUE: &str = "hunter2";

#[test]
fn id_round_trips_through_json() {
    let id = InstanceId::new();
    let text = serde_json::to_string(&id).unwrap();

    assert_eq!(serde_json::from_str::<InstanceId>(&text).unwrap(), id);
    assert_eq!(text, format!("\"{id}\""));
}

#[test]
fn id_rejects_non_ulid() {
    assert!(serde_json::from_value::<InstanceId>(json!(NOT_A_ULID)).is_err());
    assert!(NOT_A_ULID.parse::<TabId>().is_err());
}

#[test_case("a b")]
#[test_case(&"x".repeat(65))]
#[test_case("")]
#[test_case("bad/slash")]
fn id_client_parse_rejects(input: &str) {
    assert!(matches!(
        ClientId::parse(input),
        Err(Error::InvalidArgument { field, .. }) if field == "client_id"
    ));
}

#[test_case("laptop-1")]
#[test_case("a.b_c:d-e")]
#[test_case(&"x".repeat(64))]
fn id_client_parse_accepts(input: &str) {
    assert_eq!(ClientId::parse(input).unwrap().as_str(), input);
}

#[test]
fn id_client_new_is_valid() {
    let id = ClientId::new();

    assert_eq!(ClientId::parse(id.as_str()).unwrap(), id);
}

#[test]
fn validate_rejects_reserved_args() {
    for arg in [
        "--remote-debugging-port=9222",
        "--user-data-dir=/tmp/x",
        "-remote-debugging-pipe",
        "-user-data-dir=/tmp/x",
        "-profile",
        "--profile=/tmp/other",
        "-marionette",
        "--marionette",
        "-start-debugger-server",
        "--start-debugger-server=6000",
        "-new-instance",
        "-no-remote",
        "--juggler-pipe",
    ] {
        let settings = ProfileSettings {
            args: vec![arg.to_string()],
            ..ProfileSettings::default()
        };

        assert!(matches!(
            settings.validate(),
            Err(Error::InvalidArgument { field, .. }) if field == "args"
        ));
    }
}

#[test]
fn validate_accepts_unrelated_similar_switches() {
    let settings = ProfileSettings {
        args: vec![
            "--profile-directory=Default".to_string(),
            "-private".to_string(),
        ],
        ..ProfileSettings::default()
    };

    settings.validate().unwrap();
}

#[test_case("marionette.port")]
#[test_case("devtools.debugger.remote-enabled")]
#[test_case("devtools.debugger.remote-port")]
#[test_case("devtools.chrome.enabled")]
#[test_case("remote.active-protocols")]
fn validate_rejects_control_channel_prefs(key: &str) {
    let settings = ProfileSettings {
        prefs: BTreeMap::from([(key.to_string(), json!(true))]),
        ..ProfileSettings::default()
    };

    assert!(matches!(
        settings.validate(),
        Err(Error::InvalidArgument { field, .. }) if field == "settings.prefs"
    ));
}

#[test]
fn validate_rejects_brr_env() {
    let settings = ProfileSettings {
        env: BTreeMap::from([("BRR_INSTANCE_ID".to_string(), "x".to_string())]),
        ..ProfileSettings::default()
    };

    assert!(matches!(
        settings.validate(),
        Err(Error::InvalidArgument { field, .. }) if field == "env"
    ));
}

#[test]
fn validate_accepts_defaults() {
    let settings = ProfileSettings {
        args: vec!["--lang=en".to_string()],
        ..ProfileSettings::default()
    };

    assert_eq!(settings.validate(), Ok(()));
}

#[test]
fn validate_error_does_not_echo_input() {
    let settings = ProfileSettings {
        args: vec!["--user-data-dir=/home/secret".to_string()],
        ..ProfileSettings::default()
    };

    let Err(error) = settings.validate() else {
        panic!("expected an error");
    };

    assert!(!error.to_string().contains("/home/secret"));
}

#[test]
fn secret_debug_hides_value() {
    let secret = Secret::new(SECRET_VALUE);

    let shown = format!("{secret:?}");

    assert!(!shown.contains(SECRET_VALUE));
    assert_eq!(shown, "Secret(7 chars)");
    assert_eq!(secret.expose(), SECRET_VALUE);
}

#[test]
fn channel_serializes_as_one_string() {
    let channel = Channel {
        source: BuildSource::Camoufox,
        name: "latest-compatible".to_string(),
    };

    assert_eq!(
        serde_json::to_value(&channel).unwrap(),
        json!("camoufox:latest-compatible")
    );
    assert_eq!(
        serde_json::from_value::<Channel>(json!("camoufox:latest-compatible")).unwrap(),
        channel
    );
}

#[test_case("stable")]
#[test_case("nope:stable")]
#[test_case("cft:")]
fn channel_rejects_bad_string(input: &str) {
    assert!(serde_json::from_value::<Channel>(json!(input)).is_err());
}

#[test]
fn build_choice_uses_tagged_keys() {
    let choice: BuildChoice = serde_json::from_value(json!({ "channel": "cft:stable" })).unwrap();

    assert_eq!(ProfileSettings::default().build, choice);
}

#[test_case("browser,system", vec![Toolset::Browser, Toolset::System])]
#[test_case("browser", vec![Toolset::Browser])]
#[test_case("all", Toolset::ALL.to_vec())]
fn toolset_parse_list(input: &str, expected: Vec<Toolset>) {
    assert_eq!(Toolset::parse_list(input).unwrap(), expected);
}

#[test]
fn toolset_parse_list_names_valid_values() {
    let Err(Error::InvalidArgument { field, message }) = Toolset::parse_list("browser,bogus")
    else {
        panic!("expected InvalidArgument");
    };

    assert_eq!(field, "toolsets");
    assert!(message.contains("instances"));
    assert!(message.contains("comma-separated"));
}

#[test_case(None, 100)]
#[test_case(Some(0), 1)]
#[test_case(Some(5000), 1000)]
#[test_case(Some(20), 20)]
fn page_limit_is_clamped(limit: Option<u32>, expected: u32) {
    assert_eq!(Page::new(limit, None).limit, expected);
}

#[test_case(json!(5), 5_000_000)]
#[test_case(json!(5.5), 5_500_000)]
fn usd_micros_reads_numbers(input: serde_json::Value, expected: u64) {
    assert_eq!(
        serde_json::from_value::<UsdMicros>(input).unwrap(),
        UsdMicros(expected)
    );
}

#[test]
fn usd_micros_writes_decimal() {
    assert_eq!(
        serde_json::to_value(UsdMicros(5_500_000)).unwrap(),
        json!(5.5)
    );
}

#[test]
fn usd_micros_rejects_negative() {
    assert!(serde_json::from_value::<UsdMicros>(json!(-1)).is_err());
}

#[test_case("HTTPS://GitHub.com:443/login?x=1", Some("https://github.com"))]
#[test_case("http://a.test:8080/p", Some("http://a.test:8080"))]
#[test_case("http://u:p@A.test:80#f", Some("http://a.test"))]
#[test_case("https://a.test:80", Some("https://a.test:80"))]
#[test_case("https://evil.test\\@github.com/", Some("https://evil.test"))]
#[test_case("chrome://settings", None)]
#[test_case("example.com", None)]
fn site_origin_normalizes(url: &str, expected: Option<&str>) {
    assert_eq!(site_origin(url).as_deref(), expected);
}

#[test]
fn credential_schema_has_no_secrets() {
    let value = serde_json::to_value(schema_for!(Credential)).unwrap();
    let properties = value["properties"].as_object().unwrap();

    for secret in ["password", "totp", "notes"] {
        assert!(!properties.contains_key(secret), "{secret}");
    }
    assert!(properties.contains_key("totp_set"));
    assert_eq!(
        Error::Conflict {
            reason: ConflictReason::AttachDeclined,
            detail: None
        }
        .body()
        .details,
        json!({ "reason": "attach_declined" })
    );
}

fn sample_profile() -> Profile {
    Profile {
        id: ProfileId::new(),
        name: "p".to_string(),
        kind: ProfileKind::Persistent,
        template_id: None,
        template_name: None,
        settings: ProfileSettings::default(),
        pinned: false,
        notes: String::new(),
        tags: Vec::new(),
        data_dir: None,
        disk_quota_bytes: None,
        backup_schedule: None,
        backup_keep: 7,
        created_at: 0,
        updated_at: 0,
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    }
}

#[test]
fn stored_profile_without_login_status_loads() {
    let mut value = serde_json::to_value(sample_profile()).unwrap();
    value.as_object_mut().unwrap().remove("login_status");

    let profile: Profile = serde_json::from_value(value).unwrap();

    assert_eq!(profile.login_status, None);
}

#[test]
fn secret_schema_is_inline_and_write_only() {
    let mut generator = schemars::SchemaGenerator::default();
    let schema = serde_json::to_value(generator.subschema_for::<Secret>()).unwrap();

    assert_eq!(schema, json!({ "type": "string", "writeOnly": true }));
}

#[test]
fn sha256_hex_of_abc() {
    assert_eq!(
        crate::sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
