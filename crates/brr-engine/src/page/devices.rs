use std::collections::HashMap;
use std::sync::LazyLock;

use brr_core::Size;
use serde::Deserialize;

const DEVICES: &str = include_str!("../../assets/devices.json");

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Device {
    pub viewport: Size,
    #[serde(rename = "deviceScaleFactor")]
    pub scale: f64,
    #[serde(rename = "isMobile")]
    pub mobile: bool,
    #[serde(rename = "hasTouch")]
    pub touch: bool,
    #[serde(rename = "userAgent")]
    pub user_agent: String,
}

static LIST: LazyLock<HashMap<String, Device>> =
    LazyLock::new(|| serde_json::from_str(DEVICES).unwrap_or_default());

// Playwright's descriptor names, exactly as in deviceDescriptorsSource.json.
pub fn device(name: &str) -> Option<&'static Device> {
    LIST.get(name)
}
