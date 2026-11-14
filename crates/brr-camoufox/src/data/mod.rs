use std::{collections::BTreeMap, sync::LazyLock};

use serde::Deserialize;
use serde_json::Value;

// Both forms per file: the parsed Value for lookups, the text for readers that need
// the document's key order (fpgen::model::parse_ordered).
macro_rules! data_file {
    ($text:ident, $parsed:ident, $file:literal) => {
        pub const $text: &str = include_str!(concat!("../../data/", $file));
        pub static $parsed: LazyLock<Value> =
            LazyLock::new(|| serde_json::from_str($text).unwrap_or(Value::Null));
    };
}

data_file!(
    ESSENTIAL_FONTS_JSON,
    ESSENTIAL_FONTS,
    "essential-fonts.json"
);

data_file!(
    FINGERPRINT_PRESETS_JSON,
    FINGERPRINT_PRESETS,
    "fingerprint-presets.json"
);
data_file!(
    FINGERPRINT_PRESETS_V150_JSON,
    FINGERPRINT_PRESETS_V150,
    "fingerprint-presets-v150.json"
);
data_file!(FONT_BASES_JSON, FONT_BASES, "font-bases.json");
data_file!(FONT_GROUPS_JSON, FONT_GROUPS, "font-groups.json");
data_file!(FONTS_JSON, FONTS, "fonts.json");
data_file!(FPGEN_JSON, FPGEN, "fpgen.json");
data_file!(
    LAUNCHER_CONSTANTS_JSON,
    LAUNCHER_CONSTANTS,
    "launcher-constants.json"
);
data_file!(MEDIA_DEVICES_JSON, MEDIA_DEVICES, "media-devices.json");
data_file!(TERRITORY_INFO_JSON, TERRITORY_INFO, "territory-info.json");
data_file!(
    VOICE_MANIFESTS_JSON,
    VOICE_MANIFESTS,
    "voice-manifests.json"
);
data_file!(VOICE_URIS_JSON, VOICE_URIS, "voice-uris.json");

const FPGEN_MODEL_PIN_JSON: &str = include_str!("../../data/fpgen-model.json");

#[derive(Debug, Default, Deserialize)]
pub struct FpgenPin {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub files: Vec<String>,
    pub file_sha256: BTreeMap<String, String>,
    pub decompressed_sha256: BTreeMap<String, String>,
}

static FPGEN_PIN: LazyLock<FpgenPin> =
    LazyLock::new(|| serde_json::from_str(FPGEN_MODEL_PIN_JSON).unwrap_or_default());

pub fn fpgen_pin() -> &'static FpgenPin {
    &FPGEN_PIN
}

#[cfg(test)]
mod tests;
