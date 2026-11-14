use brr_core::{Error, PasswordCsvFormat};

use super::{parse_password_csv, read_records};

const CHROME: &str = include_str!("../../tests/fixtures/chrome.csv");
const FIREFOX: &str = include_str!("../../tests/fixtures/firefox.csv");

#[test]
fn chrome_export() {
    let import = parse_password_csv(CHROME, PasswordCsvFormat::Chrome).expect("parse");

    let rows: Vec<_> = import
        .logins
        .iter()
        .map(|login| {
            (
                login.site.as_str(),
                login.login_url.as_str(),
                login.username.as_str(),
                login.password.expose(),
                login.note.as_deref(),
            )
        })
        .collect();

    assert_eq!(
        rows,
        [
            (
                "https://example.com",
                "https://example.com/login",
                "alice",
                "pw1",
                None
            ),
            (
                "https://acme.test",
                "https://Acme.test:443/sign-in",
                "dan,smith",
                "pw2",
                Some("She said \"hi\"")
            ),
            (
                "http://multi.test",
                "http://multi.test/a?b=1",
                "erin",
                "pw3",
                Some("line one\nline two")
            ),
            (
                "https://plain.test",
                "https://plain.test",
                "gina",
                "pw5",
                Some("remember me")
            ),
        ]
    );
    assert_eq!(import.skipped, 2);
}

#[test]
fn firefox_export() {
    let err = parse_password_csv(FIREFOX, PasswordCsvFormat::Chrome).expect_err("not chrome");
    assert!(matches!(err, Error::InvalidArgument { ref field, .. } if field == "file"));

    let import = parse_password_csv(FIREFOX, PasswordCsvFormat::Firefox).expect("parse");

    let rows: Vec<_> = import
        .logins
        .iter()
        .map(|login| {
            (
                login.site.as_str(),
                login.username.as_str(),
                login.password.expose(),
            )
        })
        .collect();

    assert_eq!(
        rows,
        [
            ("https://example.com", "alice", "pw1"),
            ("https://acme.test:8443", "dan,smith", "p\"w2"),
        ]
    );
    assert!(import.logins.iter().all(|login| login.note.is_none()));
    assert_eq!(import.skipped, 2);
}

#[test]
fn columns_map_by_name() {
    let text = "note,password,name,username,url\r\nhi,pw,x,bob,https://a.test\r\n";

    let import = parse_password_csv(text, PasswordCsvFormat::Chrome).expect("parse");

    assert_eq!(import.logins[0].username, "bob");
    assert_eq!(import.logins[0].password.expose(), "pw");
    assert_eq!(import.logins[0].note.as_deref(), Some("hi"));
}

#[test]
fn bom_and_crlf() {
    let text = "\u{feff}url,username,password\r\nhttps://a.test,bob,pw\r\n\r\n";

    let import = parse_password_csv(text, PasswordCsvFormat::Firefox).expect("parse");

    assert_eq!(import.logins.len(), 1);
    assert_eq!(import.skipped, 0);
}

#[test]
fn error_names_columns_not_values() {
    let err = parse_password_csv("a,b\nsecret,value\n", PasswordCsvFormat::Firefox)
        .expect_err("header mismatch");

    let Error::InvalidArgument { message, .. } = err else {
        panic!("wrong error");
    };
    assert_eq!(
        message,
        "not a Firefox password export: expected columns url, username, password"
    );
}

#[test]
fn quoted_newlines_stay_in_the_field() {
    let records = read_records("a,\"b\r\nc\",d\ne,f,g").expect("records");

    assert_eq!(records, [["a", "b\r\nc", "d"], ["e", "f", "g"]]);
}

#[test]
fn unterminated_quote_is_an_error() {
    let text = "url,username,password\nhttps://a.test,bob,\"hunter2\n";

    let err = parse_password_csv(text, PasswordCsvFormat::Firefox).expect_err("unterminated");

    let Error::InvalidArgument { field, message } = err else {
        panic!("wrong error");
    };
    assert_eq!(field, "file");
    assert_eq!(message, "unterminated quoted field");
}

#[test]
fn row_wider_than_header_is_skipped() {
    let text = "url,username,password\nhttps://a.test,bob,pw,x,y,z\nhttps://b.test,eve,pw2\n";

    let import = parse_password_csv(text, PasswordCsvFormat::Firefox).expect("parse");

    assert_eq!(import.logins.len(), 1);
    assert_eq!(import.skipped, 1);
}
