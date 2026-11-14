use brr_core::capture::ResourceType;
use test_case::test_case;

use super::{PausedRequest, RequestRules, Verdict, verdict};

const IMAGE_URL: &str = "https://a/x.png";
const STYLE_URL: &str = "https://a/x.css";

fn request(url: &str, resource_type: &str) -> PausedRequest {
    PausedRequest {
        url: url.to_owned(),
        resource_type: resource_type.to_owned(),
        frame_id: "main".to_owned(),
        source_url: None,
    }
}

fn rules() -> RequestRules {
    RequestRules {
        patterns: vec!["*.png".to_owned()],
        types: vec![ResourceType::Font],
    }
}

#[test_case(IMAGE_URL, "Image", Verdict::Fail; "pattern match")]
#[test_case("https://a/x.woff", "Font", Verdict::Fail; "blocked type")]
#[test_case(STYLE_URL, "Stylesheet", Verdict::Continue; "unlisted")]
#[test_case(IMAGE_URL, "Document", Verdict::Fail; "pattern on any type")]
fn verdict_applies_rules(url: &str, resource_type: &str, expected: Verdict) {
    assert_eq!(verdict(&rules(), &request(url, resource_type)), expected);
}

#[test_case("XHR"; "xhr")]
#[test_case("Fetch"; "fetch")]
fn xhr_covers_both_cdp_kinds(resource_type: &str) {
    let rules = RequestRules {
        patterns: Vec::new(),
        types: vec![ResourceType::Xhr],
    };

    assert_eq!(
        verdict(&rules, &request(STYLE_URL, resource_type)),
        Verdict::Fail
    );
}

#[test]
fn empty_rules_continue_everything() {
    assert_eq!(
        verdict(&RequestRules::default(), &request(IMAGE_URL, "Image")),
        Verdict::Continue
    );
}
