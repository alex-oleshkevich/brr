use std::sync::LazyLock;

use serde_json::Value;
use test_case::test_case;

use super::{
    ESSENTIAL_FONTS, FINGERPRINT_PRESETS, FINGERPRINT_PRESETS_V150, FONT_BASES, FONT_GROUPS, FONTS,
    FPGEN, LAUNCHER_CONSTANTS, MEDIA_DEVICES, TERRITORY_INFO, VOICE_MANIFESTS, VOICE_URIS,
    fpgen_pin,
};

#[test_case(&ESSENTIAL_FONTS; "essential fonts")]
#[test_case(&FINGERPRINT_PRESETS; "fingerprint presets")]
#[test_case(&FINGERPRINT_PRESETS_V150; "fingerprint presets v150")]
#[test_case(&FONT_BASES; "font bases")]
#[test_case(&FONT_GROUPS; "font groups")]
#[test_case(&FONTS; "fonts")]
#[test_case(&FPGEN; "fpgen")]
#[test_case(&LAUNCHER_CONSTANTS; "launcher constants")]
#[test_case(&MEDIA_DEVICES; "media devices")]
#[test_case(&TERRITORY_INFO; "territory info")]
#[test_case(&VOICE_MANIFESTS; "voice manifests")]
#[test_case(&VOICE_URIS; "voice uris")]
fn vendored_file_parses(file: &LazyLock<Value>) {
    assert!(file.is_object() || file.is_array());
}

#[test]
fn model_pin_names_every_member() {
    let pin = fpgen_pin();

    assert_eq!(pin.sha256.len(), 64);
    assert!(pin.url.starts_with("https://"));
    for member in &pin.files {
        assert!(pin.file_sha256.contains_key(member), "{member}");
    }
    assert!(pin.decompressed_sha256.contains_key("values.dat"));
}
