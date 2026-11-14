use brr_core::{CaptchaType, Secret};
use serde_json::{Value, json};
use test_case::test_case;

use super::{APPLY_JS, apply_js};
use crate::{detect::Detected, provider::Solution};

const HOSTILE_TOKEN: &str = "x\";alert(1);'\\</script>";

fn detected(captcha_type: CaptchaType, geetest_version: Option<u8>) -> Detected {
    Detected {
        captcha_type,
        sitekey: None,
        action: None,
        data_s: None,
        enterprise: false,
        invisible: false,
        api_subdomain: None,
        gt: None,
        challenge: None,
        geetest_version,
        image_selector: None,
        input_selector: None,
    }
}

fn payload(script: &str) -> Value {
    let call = script.strip_prefix(APPLY_JS.trim_end()).unwrap();
    serde_json::from_str(call.strip_prefix('(').unwrap().strip_suffix(')').unwrap()).unwrap()
}

#[test_case(CaptchaType::RecaptchaV2, "recaptcha_v2")]
#[test_case(CaptchaType::RecaptchaV3, "recaptcha_v3")]
#[test_case(CaptchaType::Hcaptcha, "hcaptcha")]
#[test_case(CaptchaType::Turnstile, "turnstile")]
#[test_case(CaptchaType::Funcaptcha, "funcaptcha")]
fn token_is_escaped(captcha_type: CaptchaType, name: &str) {
    let script = apply_js(
        &detected(captcha_type, None),
        &Solution::Token(Secret::new(HOSTILE_TOKEN)),
    )
    .unwrap();

    let payload = payload(&script);

    assert_eq!(payload["token"], HOSTILE_TOKEN);
    assert_eq!(payload["type"], name);
}

#[test]
fn geetest_v3_maps_to_page_inputs() {
    let solution = Solution::Geetest(
        json!({"challenge": "c", "validate": "v", "seccode": "s", "evil\"]": "x"}),
    );

    let script = apply_js(&detected(CaptchaType::Geetest, Some(3)), &solution).unwrap();

    assert_eq!(
        payload(&script)["fields"],
        json!({"geetest_challenge": "c", "geetest_validate": "v", "geetest_seccode": "s"})
    );
}

#[test]
fn geetest_v4_keeps_known_fields_only() {
    let solution = Solution::Geetest(
        json!({"captcha_id": "i", "lot_number": "l", "pass_token": "p", "gen_time": "t", "captcha_output": "o", "evil": "x"}),
    );

    let script = apply_js(&detected(CaptchaType::Geetest, Some(4)), &solution).unwrap();

    assert_eq!(
        payload(&script)["fields"],
        json!({"captcha_id": "i", "lot_number": "l", "pass_token": "p", "gen_time": "t", "captcha_output": "o"})
    );
}

#[test]
fn image_has_no_script() {
    let solution = Solution::Text(Secret::new("abc"));

    assert_eq!(
        apply_js(&detected(CaptchaType::Image, None), &solution),
        None
    );
}
