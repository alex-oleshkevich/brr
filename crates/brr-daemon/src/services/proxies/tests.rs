use brr_core::ProxyScheme;

use super::parse_import as parse;

const PASSWORD_WITH_COLONS: &str = "p:with:colons";

#[test]
fn parse_import() {
    let (parsed, invalid) = parse(
        ProxyScheme::Http,
        "a.example:8080\nb.example:81:u:p:with:colons\nbad\n\nc:0",
    );

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].host, "a.example");
    assert_eq!(parsed[0].port, 8080);
    assert!(parsed[0].username.is_none());
    assert_eq!(parsed[1].username.as_deref(), Some("u"));
    assert_eq!(
        parsed[1].password.as_ref().map(|p| p.expose()),
        Some(PASSWORD_WITH_COLONS)
    );
    assert_eq!(invalid, vec![3, 5]);
}

#[test]
fn parse_import_url_forms() {
    let (parsed, invalid) = parse(
        ProxyScheme::Http,
        "socks5://u:p@c.example:1080\nhttps://d.example:443\nu2:p2@e.example:81\nftp://x:1\nhttp://u:@f.example",
    );

    assert_eq!(invalid, vec![4, 5]);
    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0].scheme, ProxyScheme::Socks5);
    assert_eq!(parsed[0].host, "c.example");
    assert_eq!(parsed[0].username.as_deref(), Some("u"));
    assert_eq!(parsed[0].password.as_ref().map(|p| p.expose()), Some("p"));
    assert_eq!(parsed[1].scheme, ProxyScheme::Https);
    assert!(parsed[1].username.is_none());
    assert_eq!(parsed[2].scheme, ProxyScheme::Http);
    assert_eq!(parsed[2].username.as_deref(), Some("u2"));
}

#[test]
fn parse_import_colon_form_keeps_an_at_sign_in_the_password() {
    let (parsed, invalid) = parse(ProxyScheme::Http, "a.example:81:user:p@ss:w0rd");

    assert!(invalid.is_empty());
    assert_eq!(parsed[0].host, "a.example");
    assert_eq!(parsed[0].port, 81);
    assert_eq!(parsed[0].username.as_deref(), Some("user"));
    assert_eq!(
        parsed[0].password.as_ref().map(|p| p.expose()),
        Some("p@ss:w0rd")
    );
}
