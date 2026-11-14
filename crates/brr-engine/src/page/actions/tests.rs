use std::time::Duration;

use brr_core::Error;
use test_case::test_case;

use super::{Point, clickable_point, glob_match, retry_wait};
use crate::page::input::{Modifier, effective, key_description, parse_combo};
use crate::page::{ClickOptions, MouseButton, Page, Target, WaitCondition};

const SQUARE: [f64; 8] = [10.0, 20.0, 30.0, 20.0, 30.0, 40.0, 10.0, 40.0];
const VIEWPORT: Point = Point { x: 800.0, y: 600.0 };
const DOT: [f64; 8] = [5.0, 5.0, 5.5, 5.0, 5.5, 5.5, 5.0, 5.5];

#[test_case("Control+Shift+A", &[Modifier::Control, Modifier::Shift], "A"; "combo")]
#[test_case("+", &[], "+"; "plus alone")]
#[test_case("Shift++", &[Modifier::Shift], "+"; "shift plus")]
#[test_case("Enter", &[], "Enter"; "named key")]
fn parses_key_combos(text: &str, modifiers: &[Modifier], key: &str) {
    let combo = parse_combo(text).unwrap();

    assert_eq!(combo.modifiers, modifiers);
    assert_eq!(combo.key.key, key);
}

#[test_case("Foo"; "unknown key")]
#[test_case("Foo+A"; "unknown modifier")]
fn rejects_key_combos(text: &str) {
    assert!(matches!(
        parse_combo(text),
        Err(Error::InvalidArgument { field, .. }) if field == "key"
    ));
}

#[test]
fn clickable_point_is_the_offset_quad_center() {
    let offsets = [Point { x: 100.0, y: 200.0 }, Point { x: 1.0, y: 2.0 }];

    let point = clickable_point(&[DOT, SQUARE], &offsets, &VIEWPORT).unwrap();

    assert_eq!(point, Point { x: 121.0, y: 232.0 });
    assert_eq!(clickable_point(&[DOT], &[], &VIEWPORT), None);
}

#[test]
fn clickable_point_is_clipped_to_the_viewport() {
    // Taller than the viewport: the visible half is clicked, not the off-screen center.
    let tall = [0.0, 0.0, 100.0, 0.0, 100.0, 2000.0, 0.0, 2000.0];

    let point = clickable_point(&[tall], &[], &VIEWPORT).unwrap();

    assert_eq!(point, Point { x: 50.0, y: 300.0 });

    let below = [0.0, 700.0, 100.0, 700.0, 100.0, 800.0, 0.0, 800.0];
    assert_eq!(clickable_point(&[below], &[], &VIEWPORT), None);
}

#[test]
fn retry_waits_follow_playwright() {
    let waits: Vec<Duration> = (0..7).map(retry_wait).collect();

    assert_eq!(
        waits,
        [0, 20, 100, 100, 500, 500, 500].map(Duration::from_millis)
    );
}

#[test_case("https://a.test/*", "https://a.test/x/y", true; "suffix run")]
#[test_case("*/done", "https://a.test/done", true; "prefix run")]
#[test_case("a*b*c", "a--b--c", true; "two runs")]
#[test_case("a*b*c", "a--c--b", false; "wrong order")]
#[test_case("https://a.test/*", "https://b.test/x", false; "other host")]
#[test_case("*", "", true; "only star")]
#[test_case("https://a/x.png", "https://a/x.png", true; "exact")]
#[test_case("*.png", "https://a/x.png?v=1", false; "query defeats suffix")]
#[test_case("a*b*b", "ab", false; "parts do not overlap")]
#[test_case("*.jpg", "https://a/x.png", false; "other suffix")]
fn matches_globs(pattern: &str, text: &str, expected: bool) {
    assert_eq!(glob_match(pattern, text), expected);
}

#[test]
fn keys_follow_the_us_layout() {
    let a = key_description("a").unwrap();
    assert_eq!(
        (a.code.as_str(), a.key_code, a.text.as_str()),
        ("KeyA", 65, "a")
    );

    let shifted = effective(key_description("KeyA").unwrap(), &[Modifier::Shift]);
    assert_eq!(shifted.text, "A");

    let control = effective(key_description("KeyA").unwrap(), &[Modifier::Control]);
    assert_eq!(control.text, "");
    assert!(key_description("\n").is_some());
}

fn assert_send<T: Send>(_: T) {}

// Handlers run on a multi-thread runtime: a Page call whose future is not provably Send breaks them.
#[allow(dead_code)]
fn retrying_calls_are_send(page: &Page) {
    let target = Target::Selector("button".to_owned());
    let options = ClickOptions {
        button: MouseButton::Left,
        modifiers: Vec::new(),
        click_count: 1,
    };
    let timeout = Duration::ZERO;

    assert_send(page.click(&target, &options, timeout));
    assert_send(page.hover(&target, timeout));
    assert_send(page.fill(&target, "", timeout));
    assert_send(page.select_option(&target, &[], timeout));
    assert_send(page.set_checked(&target, true, timeout));
    assert_send(page.drag(&target, &target, timeout));
    assert_send(page.wait_for(&WaitCondition::NetworkIdle, timeout));
}
