use brr_core::{CaptchaType, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(test)]
mod tests;

pub const DETECT_JS: &str = include_str!("detect.js");

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    #[serde(rename = "type")]
    pub captcha_type: CaptchaType,
    pub sitekey: Option<String>,
    pub action: Option<String>,
    pub data_s: Option<String>,
    pub enterprise: bool,
    pub invisible: bool,
    pub api_subdomain: Option<String>,
    // For GeeTest v4 `gt` holds the captcha_id
    pub gt: Option<String>,
    pub challenge: Option<String>,
    pub geetest_version: Option<u8>,
    pub image_selector: Option<String>,
    pub input_selector: Option<String>,
}

pub fn parse_detected(value: Value) -> Result<Option<Detected>, Error> {
    if value.is_null() {
        return Ok(None);
    }

    serde_json::from_value(value)
        .map(Some)
        .map_err(|source| Error::Internal {
            message: format!("unreadable captcha detection result: {source}"),
        })
}
