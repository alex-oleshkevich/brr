use test_case::test_case;

use super::{DEFAULT_AUTH_MESSAGE, Endpoints, MESSAGE_CAP, auth_message, endpoints};
use crate::Error;

#[test_case("https://vault.bitwarden.com", "https://identity.bitwarden.com", "https://api.bitwarden.com" ; "us cloud")]
#[test_case("https://vault.bitwarden.com/", "https://identity.bitwarden.com", "https://api.bitwarden.com" ; "us cloud trailing slash")]
#[test_case("https://vault.bitwarden.eu", "https://identity.bitwarden.eu", "https://api.bitwarden.eu" ; "eu cloud")]
#[test_case("https://vw.example.org", "https://vw.example.org/identity", "https://vw.example.org/api" ; "self hosted")]
#[test_case(" https://example.org/vault/ ", "https://example.org/vault/identity", "https://example.org/vault/api" ; "self hosted under a path")]
#[test_case("http://localhost:8080", "http://localhost:8080/identity", "http://localhost:8080/api" ; "localhost over http")]
#[test_case("http://127.0.0.1:8000", "http://127.0.0.1:8000/identity", "http://127.0.0.1:8000/api" ; "loopback over http")]
fn endpoints_map_server_url(server_url: &str, identity: &str, api: &str) {
    assert_eq!(
        endpoints(server_url).unwrap(),
        Endpoints {
            identity: identity.into(),
            api: api.into(),
        }
    );
}

#[test_case("http://vw.example.org" ; "plain http to a remote host")]
#[test_case("ftp://vw.example.org" ; "other scheme")]
#[test_case("vault.bitwarden.com" ; "no scheme")]
#[test_case("" ; "empty")]
fn endpoints_reject_bad_server_url(server_url: &str) {
    assert!(matches!(
        endpoints(server_url),
        Err(Error::InvalidServer(_))
    ));
}

#[test_case(r#"{"error":"invalid_client","error_description":"Invalid client\u001b[31m"}"#, "Invalid client[31m" ; "description without control characters")]
#[test_case(r#"{"error":"invalid_client"}"#, "invalid_client" ; "error code without description")]
#[test_case(r#"{"message":"Invalid client_id","errorModel":{"message":"Invalid client_id"},"error":"","error_description":"","object":"error"}"#, "Invalid client_id" ; "vaultwarden message")]
#[test_case(r#"{"error":"","error_description":"  "}"#, DEFAULT_AUTH_MESSAGE ; "blank fields")]
#[test_case("<html>Bad Request</html>", DEFAULT_AUTH_MESSAGE ; "not json")]
fn auth_message_reads_identity_error(body: &str, expected: &str) {
    assert_eq!(auth_message(body.as_bytes()), expected);
}

#[test]
fn auth_message_is_capped() {
    let body = format!(
        r#"{{"error_description":"{}"}}"#,
        "x".repeat(MESSAGE_CAP * 2)
    );

    assert_eq!(auth_message(body.as_bytes()).chars().count(), MESSAGE_CAP);
}
