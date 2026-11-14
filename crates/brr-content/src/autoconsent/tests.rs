use super::{AUTOCONSENT_JS, AUTOCONSENT_VERSION};

#[test]
fn bundle_matches_version_file() {
    assert!(AUTOCONSENT_JS.contains("autoconsentReceiveMessage"));
    assert_eq!(
        AUTOCONSENT_VERSION,
        include_str!("../../vendor/autoconsent/VERSION").trim()
    );
}
