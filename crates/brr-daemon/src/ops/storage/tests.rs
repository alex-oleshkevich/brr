use super::{credentials_rejected, short_cause};

#[test]
fn addressing_errors_point_to_path_style() {
    for raw in [
        "Generic S3 error: ...<Code>InvalidIpv4Address</Code>",
        "response: InvalidBucketName",
    ] {
        assert!(short_cause(raw).contains("path-style"), "{raw}");
    }
}

#[test]
fn known_causes_stay_short() {
    assert_eq!(short_cause("NoSuchBucket: gone"), "bucket not found");
}

#[test]
fn rejected_credentials_are_not_a_reachability_failure() {
    for raw in [
        "<Code>SignatureDoesNotMatch</Code>",
        "InvalidAccessKeyId: nope",
    ] {
        assert!(credentials_rejected(raw), "{raw}");
    }
    assert!(!credentials_rejected("connection refused"));
}
