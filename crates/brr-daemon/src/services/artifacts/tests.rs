use brr_storage::validate_key;
use test_case::test_case;

use super::{DEFAULT_NAME, key_name};

#[test_case("report.pdf", "report.pdf"; "plain")]
#[test_case("my file (1).pdf", "my_file__1_.pdf"; "spaces and parentheses")]
#[test_case("../../etc/passwd", "____etc_passwd"; "traversal")]
#[test_case("a..b", "a_b"; "double dot")]
#[test_case("", DEFAULT_NAME; "empty")]
#[test_case(".", DEFAULT_NAME; "dot")]
#[test_case(".report.tmp", "_report.tmp"; "leading dot")]
fn key_name_is_a_valid_key_segment(filename: &str, expected: &str) {
    let name = key_name(filename);

    assert_eq!(name, expected);
    validate_key(&format!("artifacts/id/{name}")).unwrap();
}
