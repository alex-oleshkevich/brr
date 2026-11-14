use test_case::test_case;

use super::{crx::scan_updatecheck, parse_amo_ref, parse_cws_ref};

const CWS_ID: &str = "abcdefghijklmnopabcdefghijklmnop";

#[test_case(CWS_ID, Some(CWS_ID); "bare id")]
#[test_case("https://chromewebstore.google.com/detail/some-name/abcdefghijklmnopabcdefghijklmnop", Some(CWS_ID); "new store url")]
#[test_case("https://chrome.google.com/webstore/detail/abcdefghijklmnopabcdefghijklmnop?hl=en", Some(CWS_ID); "old store url")]
#[test_case("https://evil.example/detail/x/abcdefghijklmnopabcdefghijklmnop", None; "other host")]
#[test_case("abcdefghijklmnopabcdefghijklmnoz", None; "id with a letter past p")]
fn cws_refs(reference: &str, expected: Option<&str>) {
    assert_eq!(parse_cws_ref(reference).as_deref(), expected);
}

#[test_case("ublock-origin", false, Some("ublock-origin"); "slug")]
#[test_case("https://addons.mozilla.org/en-US/firefox/addon/ublock-origin/", false, Some("ublock-origin"); "url")]
#[test_case("https://evil.example/en-US/firefox/addon/x/", false, None; "other host")]
#[test_case("ublock-origin", true, None; "slug where only a url is accepted")]
fn amo_refs(reference: &str, url_only: bool, expected: Option<&str>) {
    assert_eq!(parse_amo_ref(reference, url_only).as_deref(), expected);
}

#[test]
fn updatecheck_scan_reads_version_and_codebase() {
    let xml = r#"<gupdate><app appid="x"><updatecheck codebase="http://h/x.crx?a=1&amp;b=2" version="2.0"/></app></gupdate>"#;

    assert_eq!(
        scan_updatecheck(xml),
        Some(("2.0".to_string(), "http://h/x.crx?a=1&b=2".to_string()))
    );
    assert_eq!(
        scan_updatecheck(r#"<updatecheck status="noupdate"/>"#),
        None
    );
}
