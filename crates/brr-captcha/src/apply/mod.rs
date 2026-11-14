use brr_core::CaptchaType;
use serde_json::{Map, Value, json};

use crate::{detect::Detected, provider::Solution};

#[cfg(test)]
mod tests;

const APPLY_JS: &str = include_str!("apply.js");
const GEETEST_V4_FIELDS: [&str; 5] = [
    "captcha_id",
    "lot_number",
    "pass_token",
    "gen_time",
    "captcha_output",
];
// (field in the 2captcha solution, input the page reads)
const GEETEST_V3_FIELDS: [(&str, &str); 3] = [
    ("challenge", "geetest_challenge"),
    ("validate", "geetest_validate"),
    ("seccode", "geetest_seccode"),
];

pub fn apply_js(detected: &Detected, solution: &Solution) -> Option<String> {
    let mut payload = json!({"type": detected.captcha_type});

    match (detected.captcha_type, solution) {
        (CaptchaType::Image, _) => return None,
        (CaptchaType::Geetest, Solution::Geetest(value)) => {
            payload["fields"] = Value::Object(geetest_fields(detected, value));
        }
        (_, Solution::Token(token)) => payload["token"] = json!(token.expose()),
        _ => return None,
    }

    Some(format!("{}({payload})", APPLY_JS.trim_end()))
}

// Only known names pass: a hostile provider response must not choose the input selector
fn geetest_fields(detected: &Detected, value: &Value) -> Map<String, Value> {
    let names: Vec<(&str, &str)> = if detected.geetest_version == Some(4) {
        GEETEST_V4_FIELDS
            .iter()
            .map(|name| (*name, *name))
            .collect()
    } else {
        GEETEST_V3_FIELDS.to_vec()
    };

    names
        .into_iter()
        .filter_map(|(source, input)| Some((input.to_string(), value.get(source)?.clone())))
        .collect()
}
