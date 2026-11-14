use std::fs;

use brr_core::Error;
use serde_json::{Value, json};
use tempfile::TempDir;
use test_case::test_case;

use crate::{coherence::Config, geoip::Geo};

use super::{
    CHUNK_CODE_POINTS, apply_geo, chunk_env, is_type, prefs_json, set_webgpu, validate_config,
};

const TOTAL_CODE_POINTS: usize = 70_000;
const EMOJI: char = '\u{1F600}';
const MANY_CHUNKS: usize = 10;

fn code_points(chunk: &str) -> usize {
    chunk.chars().count()
}

#[test]
fn chunking_never_splits_a_character() {
    // The emoji is the last code point of the first chunk, and four bytes long
    let mut text = "a".repeat(CHUNK_CODE_POINTS - 1);
    text.push(EMOJI);
    text.push_str(&"b".repeat(TOTAL_CODE_POINTS - CHUNK_CODE_POINTS));
    assert_eq!(code_points(&text), TOTAL_CODE_POINTS);

    let chunks = chunk_env("CAMOU_CONFIG", &text);

    assert_eq!(chunks.len(), 3);
    assert!(
        chunks
            .iter()
            .all(|(_, chunk)| code_points(chunk) <= CHUNK_CODE_POINTS)
    );
    assert!(chunks[0].1.ends_with(EMOJI));
    let joined: String = chunks.iter().map(|(_, chunk)| chunk.as_str()).collect();
    assert_eq!(joined, text);
}

#[test]
fn chunking_numbers_the_chunks_for_the_browser_to_join_in_order() {
    let text = "x".repeat(CHUNK_CODE_POINTS * MANY_CHUNKS);

    let chunks = chunk_env("CAMOU_PREFS", &text);

    let names: Vec<&str> = chunks.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names.len(), MANY_CHUNKS);
    assert_eq!(names[0], "CAMOU_PREFS_1");
    assert_eq!(names[MANY_CHUNKS - 1], "CAMOU_PREFS_10");
}

#[test]
fn chunking_an_empty_value_gives_nothing() {
    assert!(chunk_env("CAMOU_CONFIG", "").is_empty());
}

#[test]
fn prefs_are_written_as_ascii_json() {
    let prefs = json!({"my.pref": "h\u{e9}llo \u{2713} \u{1F600} \u{7f}"});

    let text = prefs_json(prefs.as_object().unwrap());

    assert!(text.is_ascii());
    assert_eq!(
        text,
        r#"{"my.pref":"h\u00e9llo \u2713 \ud83d\ude00 \u007f"}"#
    );
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), prefs);
}

#[test_case(json!(1.0), "int", true; "an integral float is an int")]
#[test_case(json!(1.5), "int", false; "a fraction is not")]
#[test_case(json!(-1), "uint", false; "a negative is not unsigned")]
#[test_case(json!(true), "int", true; "a boolean passes the numeric checks")]
#[test_case(json!(2), "double", true; "an int is a double")]
#[test_case(json!(5), "str", false; "a number is not text")]
#[test_case(json!({}), "dict", true; "an object is a dict")]
#[test_case(json!([]), "dict", false; "an array is not")]
#[test_case(json!(1), "bool", false; "an int is not a bool")]
#[test_case(json!(1), "unheard-of", false; "an unknown type matches nothing")]
fn type_checks_follow_the_launchers(value: Value, expected: &str, ok: bool) {
    assert_eq!(is_type(&value, expected), ok);
}

#[test]
fn config_is_checked_against_the_builds_properties() {
    let build = TempDir::new().unwrap();
    fs::write(
        build.path().join("properties.json"),
        r#"[{"property": "navigator.buildID", "type": "str"}, {"property": "old.key", "type": "str", "removed": "150"}]"#,
    )
    .unwrap();
    let check = |config: Value| validate_config(config.as_object().unwrap(), build.path());

    assert!(check(json!({"navigator.buildID": "1", "old.key": 7, "not.in.this.build": 7})).is_ok());
    assert!(matches!(
        check(json!({"navigator.buildID": 5})),
        Err(Error::InvalidArgument { field, .. }) if field == "fingerprint"
    ));
}

#[test]
fn a_build_without_properties_is_missing() {
    let build = TempDir::new().unwrap();

    let err = validate_config(&Config::new(), build.path());

    assert!(matches!(err, Err(Error::BuildMissing { .. })));
}

#[test_case("win", json!({}), json!(true), Some(4); "Windows enables WebGPU and blocks the device")]
#[test_case("lin", json!({}), json!(false), None; "Linux has no WebGPU")]
#[test_case("lin", json!({"dom.webgpu.enabled": true}), json!(true), Some(4); "a Profile that enables it gets the device blocked")]
#[test_case("win", json!({"dom.webgpu.enabled": false}), json!(false), None; "a Profile that disables it is left alone")]
fn webgpu_follows_the_final_pref(os_key: &str, prefs: Value, enabled: Value, blocked: Option<i64>) {
    let mut prefs = prefs.as_object().cloned().unwrap();

    set_webgpu(&mut prefs, os_key, &Config::new());

    assert_eq!(prefs["dom.webgpu.enabled"], enabled);
    assert_eq!(
        prefs.get("gfx.blacklist.webgpu").and_then(Value::as_i64),
        blocked
    );
}

#[test_case("8.8.8.8", false, true; "IPv4 without a proxy")]
#[test_case("8.8.8.8", true, false; "IPv4 through a proxy")]
#[test_case("2a01:4f8::1", false, false; "IPv6")]
fn ipv4_only_is_for_unproxied_ipv4(ip: &str, proxied: bool, expected: bool) {
    let geo = Geo {
        ip: ip.parse().unwrap(),
        country: "US".to_owned(),
        timezone: "America/Chicago".to_owned(),
        latitude: 1.0,
        longitude: 2.0,
        locale: "en-US".to_owned(),
    };
    let mut config = Config::new();

    apply_geo(&mut config, &geo, proxied, false);

    assert_eq!(config.contains_key("network:ipv4Only"), expected);
}
