use test_case::test_case;

use super::{
    from_language, from_region, handle_locale, handle_locales, normalize_locale, pairwise_sum,
};
use crate::coherence::Config;

// numpy: np.sum of 200 x 0.1 is 20.0 pairwise, where a left-to-right sum gives 20.000000000000014
const TENTHS: usize = 200;
const UNIFORM_DRAW: f64 = 0.5;

#[test_case("fr-FR", "fr", Some("Latn"), "FR"; "language and region")]
#[test_case("zh-Hans-CN", "zh", None, "CN"; "the tag's own script is not kept")]
#[test_case("en-us", "en", Some("Latn"), "US"; "case is normalized")]
fn normalize_gives_the_language_script_and_region(
    tag: &str,
    language: &str,
    script: Option<&str>,
    region: &str,
) {
    let locale = normalize_locale(tag).unwrap();

    assert_eq!(locale.language, language);
    assert_eq!(locale.script.as_deref(), script);
    assert_eq!(locale.region.as_deref(), Some(region));
}

#[test_case("xx-invalid-tag-!!"; "not a tag")]
#[test_case("en"; "no region")]
#[test_case("xx-YY"; "unknown language")]
#[test_case("en-US-"; "empty subtag")]
fn normalize_refuses(tag: &str) {
    assert!(normalize_locale(tag).is_err(), "{tag}");
}

#[test]
fn a_territory_speaks_its_likeliest_language() {
    assert_eq!(from_region("DE", None).unwrap().as_string(), "de-DE");
    assert_eq!(from_region("JP", None).unwrap().as_string(), "ja-JP");
}

#[test_case("BE", &["nl-BE", "fr-BE", "de-BE"]; "Belgium speaks one of its official languages")]
#[test_case("ET", &["am-ET"]; "Ethiopia speaks Amharic, not English")]
fn a_territory_speaks_an_official_language(region: &str, official: &[&str]) {
    let locale = from_region(region, None).unwrap().as_string();

    assert!(official.contains(&locale.as_str()), "{locale}");
}

#[test]
fn an_alphabetic_region_must_be_a_territory() {
    assert!(normalize_locale("en-UK").is_err());
    assert_eq!(normalize_locale("en-GB").unwrap().as_string(), "en-GB");
}

#[test]
fn a_fixed_draw_picks_as_numpy_does() {
    // The launch goldens pin this draw: half of Germany's weight is past German
    assert_eq!(
        from_region("DE", Some(UNIFORM_DRAW)).unwrap().as_string(),
        "en-DE"
    );
}

#[test]
fn a_language_is_spoken_where_most_of_it_is() {
    assert_eq!(from_language("de", None).unwrap().as_string(), "de-DE");
}

#[test_case("CA", "en-CA"; "a region")]
#[test_case("de", "de-DE"; "a language")]
#[test_case("zh-Hans-CN", "zh-CN"; "a tag")]
fn handle_locale_gives_a_full_locale(text: &str, expected: &str) {
    assert_eq!(handle_locale(text, false).unwrap().as_string(), expected);
}

#[test]
fn handle_locale_can_leave_the_region_out() {
    let locale = handle_locale("de", true).unwrap();

    assert_eq!(locale.as_string(), "de");
    assert_eq!(locale.region, None);
}

#[test]
fn a_locale_list_sets_the_first_and_names_them_all() {
    let mut config = Config::new();

    handle_locales("ja-JP, en, ja-JP", &mut config).unwrap();

    assert_eq!(config["locale:language"], "ja");
    assert_eq!(config["locale:region"], "JP");
    assert_eq!(config["locale:script"], "Jpan");
    assert_eq!(config["locale:all"], "ja-JP, en");
}

#[test]
fn a_single_locale_has_no_list() {
    let mut config = Config::new();

    handle_locales("fr-FR", &mut config).unwrap();

    assert!(!config.contains_key("locale:all"));
}

#[test]
fn pairwise_sum_is_numpys() {
    assert_eq!(pairwise_sum(&[0.1; TENTHS]), 20.0);
}
