use schemars::schema_for;
use serde_json::{Value, json};
use test_case::test_case;

use super::{
    CaptureParams, ColorScheme, DEFAULT_SCREEN, DEFAULT_TIMEOUT_MS, OneshotKind, ResourceType,
    ResourceTypes, ReturnMode, css_length_in, margins_in,
};
use crate::{Error, Size};

fn params(extra: Value) -> CaptureParams {
    let mut base = json!({"url": "https://e.com"});
    base.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());

    serde_json::from_value(base).unwrap()
}

fn invalid_field(params: &CaptureParams, kind: OneshotKind) -> Option<String> {
    match params.validate(kind) {
        Err(Error::InvalidArgument { field, .. }) => Some(field),
        _ => None,
    }
}

#[test]
fn query_and_body_forms_agree() {
    let query: CaptureParams = serde_json::from_value(json!({
        "url": "https://e.com", "screen": "800x600", "theme": "dark", "full_page": true,
        "block": "image,font", "headers": "{\"X-A\":\"1\"}", "return": "url"
    }))
    .unwrap();
    let body: CaptureParams = serde_json::from_value(json!({
        "url": "https://e.com", "screen": {"width": 800, "height": 600}, "theme": "dark",
        "full_page": true, "block": ["image", "font"], "headers": {"X-A": "1"}, "return": "url"
    }))
    .unwrap();

    assert_eq!(
        query.screen(),
        Size {
            width: 800,
            height: 600
        }
    );
    assert_eq!(query.theme, Some(ColorScheme::Dark));
    assert_eq!(
        query.block,
        Some(ResourceTypes(vec![ResourceType::Image, ResourceType::Font]))
    );
    assert_eq!(query.headers.as_ref().unwrap().0["X-A"], "1");
    assert_eq!(query.return_mode, Some(ReturnMode::Url));
    assert_eq!(query, body);
}

#[test]
fn query_text_forms_parse_area_and_cookies() {
    let cookie = json!([{
        "name": "a", "value": "b", "domain": "e.com", "path": "/", "expires": -1,
        "httpOnly": false, "secure": true, "sameSite": "Lax"
    }]);
    let query = params(json!({"area": "1,2,30.5,40", "cookies": cookie.to_string()}));
    let body = params(json!({"area": "1,2,30.5,40", "cookies": cookie}));

    assert_eq!(query.area.as_ref().unwrap().width, 30.5);
    assert_eq!(query, body);
}

#[test]
fn bad_json_text_names_the_field() {
    let error = serde_json::from_value::<CaptureParams>(
        json!({"url": "https://e.com", "headers": "{nope"}),
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("headers"), "{error}");
}

#[test_case(json!({"bogus": 1}); "unknown key")]
#[test_case(json!({"screen": "800"}); "screen without height")]
#[test_case(json!({"screen": "0x600"}); "screen zero side")]
#[test_case(json!({"screen": "20000x600"}); "screen too large")]
#[test_case(json!({"area": "1,2,0,4"}); "area zero width")]
#[test_case(json!({"area": "1,2,3"}); "area three numbers")]
#[test_case(json!({"block": "image,video"}); "unknown resource type")]
fn validate_rejects_malformed_input(extra: Value) {
    let mut body = json!({"url": "https://e.com"});
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());

    assert!(serde_json::from_value::<CaptureParams>(body).is_err());
}

#[test_case(json!({"selector": "#a"}), OneshotKind::Pdf, "selector"; "selector on pdf")]
#[test_case(json!({"format": "jpeg", "transparent": true}), OneshotKind::Screenshot, "transparent"; "transparent jpeg")]
#[test_case(json!({"quality": 80}), OneshotKind::Screenshot, "quality"; "quality with png")]
#[test_case(json!({"quality": 0, "format": "jpeg"}), OneshotKind::Screenshot, "quality"; "quality zero")]
#[test_case(json!({"area": "0,0,10,10", "selector": "#a"}), OneshotKind::Screenshot, "area"; "area with selector")]
#[test_case(json!({"area": "0,0,10,10", "full_page": true}), OneshotKind::Screenshot, "area"; "area with full page")]
#[test_case(json!({"paper": "B9"}), OneshotKind::Pdf, "paper"; "unknown paper")]
#[test_case(json!({"margin": "1mm 2mm"}), OneshotKind::Pdf, "margin"; "two margins")]
#[test_case(json!({"url": "file:///etc/passwd"}), OneshotKind::Screenshot, "url"; "file url")]
#[test_case(json!({"url": "e.com"}), OneshotKind::Screenshot, "url"; "no scheme")]
#[test_case(json!({"scale": 5}), OneshotKind::Screenshot, "scale"; "scale high")]
#[test_case(json!({"pdf_scale": 3}), OneshotKind::Pdf, "pdf_scale"; "pdf scale high")]
#[test_case(json!({"pages": "1-"}), OneshotKind::Pdf, "pages"; "bad pages")]
#[test_case(json!({"timeout": 0}), OneshotKind::Screenshot, "timeout"; "timeout zero")]
#[test_case(json!({"timeout": 300001}), OneshotKind::Screenshot, "timeout"; "timeout high")]
#[test_case(json!({"delay": 40000}), OneshotKind::Screenshot, "delay"; "delay past default timeout")]
#[test_case(json!({"delay": 5000, "timeout": 1000}), OneshotKind::Screenshot, "delay"; "delay past timeout")]
#[test_case(json!({"paper": "A4"}), OneshotKind::Screenshot, "paper"; "pdf field on screenshot")]
#[test_case(json!({"format": "png"}), OneshotKind::Pdf, "format"; "image field on pdf")]
fn validate_rejects(extra: Value, kind: OneshotKind, field: &str) {
    assert_eq!(invalid_field(&params(extra), kind).as_deref(), Some(field));
}

#[test]
fn validate_accepts_good_requests() {
    assert_eq!(
        params(json!({"url": "DATA:text/html,hi", "format": "jpeg", "quality": 80}))
            .validate(OneshotKind::Screenshot),
        Ok(())
    );
    assert_eq!(
        params(json!({"paper": "a4", "margin": "10mm", "pages": "1-3,5", "pdf_scale": 1.5}))
            .validate(OneshotKind::Pdf),
        Ok(())
    );
    assert_eq!(params(json!({})).validate(OneshotKind::Markdown), Ok(()));
}

#[test]
fn query_numbers_become_pdf_text() {
    let query = params(json!({"pages": 1, "margin": 0}));

    assert_eq!(query.pages.as_deref(), Some("1"));
    assert_eq!(query.margin.as_deref(), Some("0"));
    assert_eq!(query.validate(OneshotKind::Pdf), Ok(()));
}

#[test]
fn defaults_apply() {
    let params = params(json!({}));

    assert_eq!(params.screen(), DEFAULT_SCREEN);
    assert_eq!(params.timeout().as_millis() as u64, DEFAULT_TIMEOUT_MS);
    assert_eq!(params.pdf(), Default::default());
}

#[test]
fn css_lengths() {
    let near = |actual: f64, expected: f64| assert!((actual - expected).abs() < 1e-4, "{actual}");

    near(css_length_in("10mm").unwrap(), 0.3937);
    near(css_length_in("96px").unwrap(), 1.0);
    near(css_length_in("0").unwrap(), 0.0);

    let four = margins_in("1in 0 2cm 96px").unwrap();
    for (actual, expected) in four.iter().zip([1.0, 0.0, 0.7874, 1.0]) {
        near(*actual, expected);
    }
    for actual in margins_in("10mm").unwrap() {
        near(actual, 0.3937);
    }

    for bad in ["5pt", "5", "-1mm", "mm", ""] {
        assert!(
            matches!(css_length_in(bad), Err(Error::InvalidArgument { .. })),
            "{bad}"
        );
    }
    assert!(margins_in("1mm 2mm").is_err());
}

#[test]
fn schema_matches_accepted_text() {
    let schema = serde_json::to_value(schema_for!(CaptureParams)).unwrap();

    for field in ["headers", "cookies", "block"] {
        let text = schema["properties"][field].to_string();
        let defs = schema["$defs"].to_string();

        assert!(
            text.contains("anyOf") || defs.contains("anyOf"),
            "{field}: {text}"
        );
    }
    assert!(schema.to_string().contains("^[0-9]+x[0-9]+$"));
    assert!(schema["properties"].get("return").is_some());
}

#[test]
fn content_cleanup_fields_default_to_the_profile_setting() {
    let bare = params(json!({}));
    let set = params(json!({"block_ads": true, "dismiss_cookies": false}));

    assert_eq!((bare.block_ads, bare.dismiss_cookies), (None, None));
    assert_eq!(
        (set.block_ads, set.dismiss_cookies),
        (Some(true), Some(false))
    );
}
