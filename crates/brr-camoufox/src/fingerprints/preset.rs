use brr_core::Error;
use serde_json::Value;

use super::{
    config::ConfigExt, fonts, identity_seed, os_name_of_platform, replace_versions, voices,
};
use crate::{
    coherence::Config,
    data::{FINGERPRINT_PRESETS, FINGERPRINT_PRESETS_V150},
    pycompat,
    pyrandom::PyRandom,
};

// Tokens that name the machine rather than the platform
// Firefox major version at which the v150 preset bundle becomes preferred
pub const PRESETS_V150_MIN_FF: i64 = 149;
const APP_VERSION_DROPPED: [&str; 4] = ["Win64", "x64", "Mobile", "Tablet"];

static EMPTY: Vec<Value> = Vec::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetFile {
    Current,
    V150,
}

impl PresetFile {
    pub fn name(self) -> &'static str {
        match self {
            PresetFile::Current => "fingerprint-presets.json",
            PresetFile::V150 => "fingerprint-presets-v150.json",
        }
    }

    fn bundle(self) -> &'static Value {
        match self {
            PresetFile::Current => &FINGERPRINT_PRESETS,
            PresetFile::V150 => &FINGERPRINT_PRESETS_V150,
        }
    }
}

// The bundled-presets file for a Firefox version (the v150 bundle from its minimum version up)
pub fn select_presets_file(ff_version: Option<&str>) -> PresetFile {
    let major = ff_version
        .filter(|version| !version.is_empty())
        .and_then(|version| version.split('.').next())
        .and_then(|major| major.trim().parse::<i64>().ok())
        .unwrap_or(0);
    if major >= PRESETS_V150_MIN_FF {
        PresetFile::V150
    } else {
        PresetFile::Current
    }
}

fn preset_key(os: &str) -> &str {
    match os {
        "win" => "windows",
        "mac" => "macos",
        "lin" => "linux",
        other => other,
    }
}

// A random preset for the given OSes (all three when none), or None when none is bundled
pub fn get_random_preset(
    os: &[&str],
    ff_version: Option<&str>,
    rng: &mut PyRandom,
) -> Option<&'static Value> {
    let bundle = select_presets_file(ff_version).bundle();
    let all = ["macos", "windows", "linux"];
    let keys: Vec<&str> = if os.is_empty() {
        all.to_vec()
    } else {
        os.iter().map(|os| preset_key(os)).collect()
    };

    let candidates: Vec<&Value> = keys
        .iter()
        .flat_map(|key| bundle["presets"][key].as_array().unwrap_or(&EMPTY))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    Some(*rng.choice(&candidates))
}

// The appVersion Firefox reports for a browser sending this user agent: "5.0 (<OS tokens>)"
// without the architecture, the Gecko revision, or the Windows build number
pub fn app_version_from_user_agent(user_agent: Option<&str>) -> Option<String> {
    let block = user_agent?.strip_prefix("Mozilla/5.0 (")?;
    let block = &block[..block.find(')')?];

    let kept: Vec<&str> = block
        .split(';')
        .map(str::trim)
        .filter(|token| {
            !(token.starts_with("rv:")
                || APP_VERSION_DROPPED.contains(token)
                || token.starts_with("Linux ")
                || token.starts_with("Intel Mac OS X"))
        })
        .map(|token| {
            if token.starts_with("Windows") {
                "Windows"
            } else {
                token
            }
        })
        .collect();
    (!kept.is_empty()).then(|| format!("5.0 ({})", kept.join("; ")))
}

// Set navigator.appVersion the way Firefox computes it: from the userAgent. Left unset it falls
// through to the host's value; taken from fpgen it is a draw of its own, and fpgen pairs some
// Linux user agents with "5.0 (Windows)".
pub fn set_app_version(config: &mut Config) {
    let user_agent = config
        .get("navigator.userAgent")
        .map_or_else(String::new, pycompat::str);
    if let Some(derived) = app_version_from_user_agent(Some(&user_agent)) {
        config.set("navigator.appVersion", Value::from(derived));
    }
}

fn oscpu_from_platform(platform: &str) -> Option<&'static str> {
    match platform {
        "MacIntel" => Some("Intel Mac OS X 10.15"),
        "Win32" => Some("Windows NT 10.0; Win64; x64"),
        _ if platform.contains("Linux") || platform.contains("linux") => Some("Linux x86_64"),
        _ => None,
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value[key].as_str().filter(|text| !text.is_empty())
}

// Convert a real fingerprint preset to CAMOU_CONFIG format. `salt` (identity_salt) keys the
// font and voice draws.
pub fn from_preset(preset: &Value, ff_version: Option<&str>, salt: u64) -> Result<Config, Error> {
    let mut config = Config::new();

    let nav = &preset["navigator"];
    if let Some(user_agent) = text(nav, "userAgent") {
        let user_agent = match ff_version.filter(|version| !version.is_empty()) {
            Some(version) => {
                let user_agent = replace_versions(user_agent, "Firefox/", version);
                replace_versions(&user_agent, "rv:", version)
            }
            None => user_agent.to_owned(),
        };
        config.set("navigator.userAgent", Value::from(user_agent));
    }
    if pycompat::truthy(&nav["platform"]) {
        config.set("navigator.platform", nav["platform"].clone());
    }
    if pycompat::truthy(&nav["hardwareConcurrency"]) {
        config.set(
            "navigator.hardwareConcurrency",
            nav["hardwareConcurrency"].clone(),
        );
    }
    if pycompat::truthy(&nav["oscpu"]) {
        config.set("navigator.oscpu", nav["oscpu"].clone());
    } else if let Some(oscpu) = text(nav, "platform").and_then(oscpu_from_platform) {
        config.set("navigator.oscpu", Value::from(oscpu));
    }
    set_app_version(&mut config);
    if let Some(touch_points) = nav.get("maxTouchPoints") {
        config.set("navigator.maxTouchPoints", touch_points.clone());
    }

    let screen = &preset["screen"];
    for key in ["width", "height"] {
        if pycompat::truthy(&screen[key]) {
            config.set(&format!("screen.{key}"), screen[key].clone());
        }
    }
    if pycompat::truthy(&screen["colorDepth"]) {
        config.set("screen.colorDepth", screen["colorDepth"].clone());
        config.set("screen.pixelDepth", screen["colorDepth"].clone());
    }
    for key in ["availWidth", "availHeight"] {
        if pycompat::truthy(&screen[key]) {
            config.set(&format!("screen.{key}"), screen[key].clone());
        }
    }

    let webgl = &preset["webgl"];
    if pycompat::truthy(&webgl["unmaskedVendor"]) {
        config.set("webGl:vendor", webgl["unmaskedVendor"].clone());
    }
    if pycompat::truthy(&webgl["unmaskedRenderer"]) {
        config.set("webGl:renderer", webgl["unmaskedRenderer"].clone());
    }

    if pycompat::truthy(&preset["timezone"]) {
        config.set("timezone", preset["timezone"].clone());
    }

    let target_os = os_name_of_platform(nav["platform"].as_str().unwrap_or_default());
    let seed = u128::from(identity_seed(&config, salt));
    let font_list =
        fonts::generate_random_font_subset(target_os, &mut PyRandom::seed_wide(seed), false, None);
    config.set("fonts", font_list.into());
    let voice_list =
        voices::generate_random_voice_subset(target_os, None, &mut PyRandom::seed_wide(seed))?;
    config.set("voices", Value::Array(voice_list));
    Ok(config)
}
