use brr_core::{CookieInput, SameSite};
use test_case::test_case;

use super::cookie_from_input;

fn input(domain: Option<&str>, url: Option<&str>) -> CookieInput {
    CookieInput {
        name: "sid".to_owned(),
        value: "1".to_owned(),
        domain: domain.map(str::to_owned),
        url: url.map(str::to_owned),
        path: None,
        expires: None,
        http_only: None,
        secure: None,
        same_site: None,
    }
}

#[test_case(None, Some("https://example.com/a"), "example.com", true; "https url")]
#[test_case(None, Some("http://example.com"), "example.com", false; "http url")]
#[test_case(Some(".other.org"), Some("https://example.com"), ".other.org", true; "domain wins")]
#[test_case(Some("example.com"), None, "example.com", false; "domain only")]
fn defaults_fill_the_rest(domain: Option<&str>, url: Option<&str>, host: &str, secure: bool) {
    let cookie = cookie_from_input(input(domain, url)).unwrap();

    assert_eq!(cookie.domain, host);
    assert_eq!(cookie.path, "/");
    assert_eq!(cookie.expires, -1.0);
    assert!(!cookie.http_only);
    assert_eq!(cookie.secure, secure);
    assert_eq!(cookie.same_site, SameSite::Lax);
}

#[test_case(None, None; "neither")]
#[test_case(None, Some("not a url"); "bad url")]
fn scope_is_required(domain: Option<&str>, url: Option<&str>) {
    assert!(cookie_from_input(input(domain, url)).is_err());
}
