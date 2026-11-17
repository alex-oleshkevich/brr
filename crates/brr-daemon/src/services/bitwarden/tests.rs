use brr_bitwarden::Error as BitwardenError;
use brr_core::Error;
use test_case::test_case;

use super::to_error;

const SERVER: &str = "https://vault.example";

#[test_case(BitwardenError::InvalidServer("x".into()), "server_url")]
#[test_case(BitwardenError::Auth("invalid_client".into()), "client_secret")]
#[test_case(BitwardenError::WrongPassword, "master_password")]
#[test_case(BitwardenError::Unsupported("KDF 7".into()), "client_id")]
fn connect_errors_name_the_field(error: BitwardenError, expected: &str) {
    match to_error(error, SERVER) {
        Error::InvalidArgument { field, .. } => assert_eq!(field, expected),
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test_case(BitwardenError::Network("refused".into()))]
#[test_case(BitwardenError::Protocol("no Key".into()))]
fn server_trouble_names_the_server(error: BitwardenError) {
    match to_error(error, SERVER) {
        Error::DependencyMissing { name, .. } => assert!(name.contains(SERVER)),
        other => panic!("expected DependencyMissing, got {other:?}"),
    }
}

#[test]
fn missing_item_is_not_found() {
    let error = to_error(BitwardenError::NotFound("item-1".into()), SERVER);

    assert_eq!(
        error,
        Error::NotFound {
            what: "Bitwarden item item-1".to_owned()
        }
    );
}
