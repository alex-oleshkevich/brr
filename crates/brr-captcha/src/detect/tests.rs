use std::{
    path::{Path, PathBuf},
    process::Command,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use brr_core::{CaptchaType, Error};
use serde_json::{Value, json};

use super::{DETECT_JS, parse_detected};

const TURNSTILE_KEY: &str = "0x4AAA";
const CHROME_ENV: &str = "BRR_TEST_CHROME";

#[test]
fn parses_turnstile() {
    let value = json!({"type": "turnstile", "sitekey": TURNSTILE_KEY, "enterprise": false, "invisible": false});

    let detected = parse_detected(value).unwrap().unwrap();

    assert_eq!(detected.captcha_type, CaptchaType::Turnstile);
    assert_eq!(detected.sitekey.as_deref(), Some(TURNSTILE_KEY));
    assert_eq!(detected.action, None);
}

#[test]
fn null_is_no_captcha() {
    assert_eq!(parse_detected(Value::Null).unwrap(), None);
}

#[test]
fn unknown_type_is_internal() {
    let value = json!({"type": "mystery", "enterprise": false, "invisible": false});

    assert!(matches!(parse_detected(value), Err(Error::Internal { .. })));
}

#[test]
fn script_is_an_iife_expression() {
    assert!(DETECT_JS.trim_start().starts_with("(function"));
    assert!(DETECT_JS.trim_end().ends_with("})()"));
}

fn detect_in_chrome(chrome: &Path, body: &str) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("page.html");
    // The result is base64 so HTML escaping in the serialized DOM cannot alter it
    let html = format!(
        "<html><body>{body}<pre id=\"out\"></pre><script>document.getElementById('out').textContent = btoa(JSON.stringify({DETECT_JS}));</script></body></html>"
    );
    std::fs::write(&page, html).unwrap();

    let output = Command::new(chrome)
        .args(["--headless", "--no-sandbox", "--disable-gpu", "--dump-dom"])
        .arg(format!(
            "--user-data-dir={}",
            dir.path().join("profile").display()
        ))
        .arg(format!("file://{}", page.display()))
        .output()
        .unwrap();

    let dom = String::from_utf8(output.stdout).unwrap();
    let encoded = dom
        .split_once("<pre id=\"out\">")
        .and_then(|(_, rest)| rest.split_once("</pre>"))
        .map(|(encoded, _)| encoded)
        .unwrap();
    serde_json::from_slice(&STANDARD.decode(encoded).unwrap()).unwrap()
}

#[allow(clippy::disallowed_macros)]
#[test]
fn detect_js_in_chrome() {
    let Some(chrome) = std::env::var_os(CHROME_ENV).map(PathBuf::from) else {
        eprintln!("skipped: {CHROME_ENV} is not set");
        return;
    };

    let v2 = detect_in_chrome(
        &chrome,
        r#"<div class="g-recaptcha" data-sitekey="K2" data-size="invisible" data-s="S"></div>"#,
    );
    assert_eq!(v2["type"], "recaptcha_v2");
    assert_eq!(v2["sitekey"], "K2");
    assert_eq!(v2["invisible"], true);
    assert_eq!(v2["data_s"], "S");

    let v3_with_badge = detect_in_chrome(
        &chrome,
        r#"<script src="https://www.google.com/recaptcha/api.js?render=KEY3"></script>
        <iframe src="https://www.google.com/recaptcha/api2/anchor?k=KEY3&size=invisible"></iframe>
        <button data-action="login">x</button>"#,
    );
    assert_eq!(v3_with_badge["type"], "recaptcha_v3");
    assert_eq!(v3_with_badge["sitekey"], "KEY3");
    assert_eq!(v3_with_badge["action"], "login");

    let empty_turnstile = detect_in_chrome(
        &chrome,
        r#"<div class="cf-turnstile" data-sitekey=""></div>"#,
    );
    assert_eq!(empty_turnstile["type"], "turnstile");
    assert_eq!(empty_turnstile["sitekey"], Value::Null);

    assert_eq!(
        detect_in_chrome(&chrome, "<p>nothing here</p>"),
        Value::Null
    );
}
