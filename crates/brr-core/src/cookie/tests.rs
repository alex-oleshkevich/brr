use serde_json::json;

use super::{Cookie, SameSite, from_json, from_netscape, to_netscape, to_storage_state};
use crate::Error;

fn storage_state() -> String {
    json!({
        "cookies": [
            {"name": "sid", "value": "abc", "domain": ".example.com", "path": "/",
             "expires": 1893456000.0, "httpOnly": true, "secure": true, "sameSite": "Strict"},
            {"name": "pref", "value": "dark", "domain": "example.com", "path": "/app",
             "expires": -1, "httpOnly": false, "secure": false, "sameSite": "None"}
        ],
        "origins": []
    })
    .to_string()
}

#[test]
fn storage_state_to_netscape_and_back() {
    let cookies = from_json(&storage_state()).unwrap();
    let netscape = to_netscape(&cookies);
    let back = from_netscape(&netscape).unwrap();

    assert!(netscape.starts_with("# Netscape HTTP Cookie File\n"));
    assert!(netscape.contains("#HttpOnly_.example.com\tTRUE\t/\tTRUE\t1893456000\tsid\tabc\n"));
    assert!(netscape.contains("example.com\tFALSE\t/app\tFALSE\t0\tpref\tdark\n"));
    assert_eq!(back.len(), 2);

    for (original, imported) in cookies.iter().zip(&back) {
        assert_eq!(imported.name, original.name);
        assert_eq!(imported.value, original.value);
        assert_eq!(imported.domain, original.domain);
        assert_eq!(imported.path, original.path);
        assert_eq!(imported.secure, original.secure);
        assert_eq!(imported.http_only, original.http_only);
        assert_eq!(imported.expires, original.expires);
        assert_eq!(imported.same_site, SameSite::Lax);
    }
}

#[test]
fn from_json_accepts_bare_array_and_round_trips_storage_state() {
    let cookies = from_json(&storage_state()).unwrap();
    let bare = serde_json::to_string(&cookies).unwrap();

    assert_eq!(from_json(&bare).unwrap(), cookies);
    assert_eq!(from_json(&to_storage_state(&cookies)).unwrap(), cookies);
    assert!(to_storage_state(&cookies).contains("\"origins\": []"));
    assert!(matches!(
        from_json("{\"nope\": 1}"),
        Err(Error::InvalidArgument { field, .. }) if field == "data"
    ));
}

#[test]
fn netscape_skips_comments_and_blank_lines() {
    let text = "# Netscape HTTP Cookie File\n# comment\n\n.a.com\tTRUE\t/\tFALSE\t0\tk\tv\n";
    let cookies = from_netscape(text).unwrap();

    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].expires, -1.0);
    assert!(!cookies[0].http_only);
}

#[test]
fn netscape_wrong_field_count_names_the_line() {
    let text = "# header\n.a.com\tTRUE\t/\tFALSE\t0\tk\tv\n.a.com\tTRUE\t/\tFALSE\t0\tk\n";

    assert_eq!(
        from_netscape(text),
        Err(Error::InvalidArgument {
            field: "data".to_string(),
            message: "line 3: expected 7 tab-separated fields".to_string(),
        })
    );
}

#[test]
fn netscape_rejects_non_numeric_expiry() {
    assert!(matches!(
        from_netscape(".a.com\tTRUE\t/\tFALSE\tsoon\tk\tv"),
        Err(Error::InvalidArgument { field, message }) if field == "data" && message.starts_with("line 1:")
    ));
}

#[test]
fn cookie_json_uses_playwright_names() {
    let cookie = &from_json(&storage_state()).unwrap()[0];
    let value = serde_json::to_value(cookie).unwrap();

    assert_eq!(value["httpOnly"], true);
    assert_eq!(value["sameSite"], "Strict");
    assert_eq!(*cookie, serde_json::from_value::<Cookie>(value).unwrap());
}
