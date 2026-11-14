use serde_json::{Value, json};
use test_case::test_case;

use super::{Config, apply, drop_incoherent_source_values, gpu_fits_os, validate};
use crate::pycompat;

const INTEL_MAC_GPU: &str = "Intel(R) HD Graphics 400, or similar";
const APPLE_GPU: &str = "Apple M1, or similar";
const ANGLE_GPU: &str =
    "ANGLE (NVIDIA, NVIDIA GeForce GTX 980 Direct3D11 vs_5_0 ps_5_0), or similar";

fn config(value: Value) -> Config {
    value.as_object().cloned().unwrap_or_default()
}

fn rules(violations: &[super::Violation]) -> Vec<&str> {
    violations.iter().map(|violation| violation.rule).collect()
}

#[test_case(2.5, 2; "half rounds to even down")]
#[test_case(3.5, 4; "half rounds to even up")]
#[test_case(-0.5, 0; "negative half rounds to zero")]
#[test_case(2.6, 3; "above half rounds up")]
fn round_is_bankers(value: f64, expected: i64) {
    assert_eq!(pycompat::round(value), expected);
}

#[test_case(-2.7, -2; "negative truncates toward zero")]
#[test_case(2.7, 2; "positive truncates down")]
fn int_truncates(value: f64, expected: i64) {
    assert_eq!(pycompat::int(value), expected);
}

#[test]
fn canon_unifies_integral_floats_and_big_ints() {
    let canonical = pycompat::canon(&json!([1.0, 9_007_199_254_740_992_u64, 0.5, 7]));

    assert!(canonical[0].is_u64());
    assert_eq!(canonical[0], json!(1));
    assert!(canonical[1].is_f64());
    assert!(canonical[2].is_f64());
    assert!(canonical[3].is_u64());
}

#[test_case(Value::Null, "None"; "none")]
#[test_case(json!(true), "True"; "bool")]
#[test_case(json!(2.0), "2.0"; "integral float")]
#[test_case(json!(1e16), "1e+16"; "large float")]
#[test_case(json!(1.5e-7), "1.5e-07"; "small float")]
#[test_case(json!(["a'b", 1]), "[\"a'b\", 1]"; "list with quote")]
fn str_matches_python(value: Value, expected: &str) {
    assert_eq!(pycompat::str(&value), expected);
}

#[test_case(APPLE_GPU, "mac", true; "apple on mac")]
#[test_case(ANGLE_GPU, "mac", false; "angle on mac")]
#[test_case(ANGLE_GPU, "win", true; "angle on windows")]
#[test_case(INTEL_MAC_GPU, "win", false; "bare renderer on windows")]
#[test_case(APPLE_GPU, "lin", false; "apple on linux")]
#[test_case(ANGLE_GPU, "bsd", true; "unknown os")]
#[test_case("", "win", true; "empty renderer")]
fn gpu_fits_os_by_platform(renderer: &str, target_os: &str, expected: bool) {
    assert_eq!(gpu_fits_os(Some(renderer), target_os), expected);
}

#[test]
fn validate_reports_every_broken_rule() {
    let broken = config(json!({
        "webGl:renderer": ANGLE_GPU,
        "navigator.maxTouchPoints": 5,
        "screen.width": 1280,
        "screen.height": 800,
    }));

    let violations = validate(&broken, "mac");

    assert_eq!(rules(&violations), ["gpu-matches-os", "touch-points"]);
    assert_eq!(
        violations[1].detail,
        "macOS identity with maxTouchPoints 5; no Mac has a touchscreen"
    );
}

#[test]
fn apply_repairs_and_returns_what_is_left() {
    let mut broken = config(json!({
        "webGl:renderer": "Radeon HD 3200 Graphics, or similar",
        "navigator.maxTouchPoints": 25,
        "window.devicePixelRatio": 1.4,
        "window.innerHeight": 900,
        "window.outerHeight": 950,
        "screen.height": 1200,
        "screen.width": 1920,
    }));

    let left = apply(&mut broken, "lin");

    assert!(left.is_empty());
    assert_eq!(broken["navigator.maxTouchPoints"], json!(0));
    assert_eq!(broken["window.devicePixelRatio"], json!(1.5));
    assert_eq!(broken["window.outerHeight"], json!(986));
}

#[test]
fn apple_silicon_cores_snap_up_to_a_shipped_count() {
    let mut machine = config(json!({
        "webGl:renderer": APPLE_GPU,
        "navigator.hardwareConcurrency": 9,
    }));

    apply(&mut machine, "mac");

    assert_eq!(machine["navigator.hardwareConcurrency"], json!(10));
}

#[test]
fn dropping_removes_both_webgl_keys() {
    let mut preset = config(json!({
        "webGl:renderer": APPLE_GPU,
        "webGl:vendor": "Apple",
        "screen.width": 1920,
    }));

    let dropped = drop_incoherent_source_values(&mut preset, "win");

    assert_eq!(rules(&dropped), ["gpu-matches-os"]);
    assert!(!preset.contains_key("webGl:renderer"));
    assert!(!preset.contains_key("webGl:vendor"));
    assert!(preset.contains_key("screen.width"));
}
