use serde_json::json;
use test_case::test_case;

use crate::{
    Capability, ClientId, ConflictReason, Error, ErrorBody, LoginFailure, QuotaScope, range_message,
};

type Expected = (&'static str, u16, u8);

const INVALID_ARGUMENT: Expected = ("InvalidArgument", 400, 2);
const NOT_FOUND: Expected = ("NotFound", 404, 3);
const CONFLICT: Expected = ("Conflict", 409, 4);
const INVALID_TRANSITION: Expected = ("InvalidTransition", 409, 5);
const LEASE_HELD: Expected = ("LeaseHeld", 423, 6);
const LEASE_LOST: Expected = ("LeaseLost", 410, 7);
const CAP_EXCEEDED: Expected = ("CapExceeded", 429, 8);
const QUOTA_EXCEEDED: Expected = ("QuotaExceeded", 507, 9);
const DOWNLOAD_TOO_LARGE: Expected = ("DownloadTooLarge", 413, 10);
const REQUEST_TOO_LARGE: Expected = ("RequestTooLarge", 413, 2);
const BUILD_MISSING: Expected = ("BuildMissing", 424, 11);
const ENGINE_UNSUPPORTED: Expected = ("EngineUnsupported", 422, 12);
const DEPENDENCY_MISSING: Expected = ("DependencyMissing", 503, 13);
const PROXY_UNHEALTHY: Expected = ("ProxyUnhealthy", 502, 14);
const NAVIGATION_FAILED: Expected = ("NavigationFailed", 502, 15);
const ELEMENT_NOT_FOUND: Expected = ("ElementNotFound", 422, 16);
const STALE_REF: Expected = ("StaleRef", 422, 16);
const EVALUATION_FAILED: Expected = ("EvaluationFailed", 422, 17);
const LOGIN_FAILED: Expected = ("LoginFailed", 422, 18);
const CAPTCHA_BUDGET_EXCEEDED: Expected = ("CaptchaBudgetExceeded", 402, 19);
const CAPTCHA_FAILED: Expected = ("CaptchaFailed", 422, 20);
const STORAGE_UNAVAILABLE: Expected = ("StorageUnavailable", 503, 21);
const TIMEOUT: Expected = ("Timeout", 504, 22);
const BITWARDEN_LOCKED: Expected = ("BitwardenLocked", 409, 23);
const INTERNAL: Expected = ("Internal", 500, 1);

const CONFLICT_PROFILE_LOCKED_MESSAGE: &str =
    "profile is already running in another instance; stop it or use an ephemeral profile";
const UNKNOWN_CODE: &str = "FromTheFuture";

#[test_case(Error::InvalidArgument { field: "f".into(), message: "m".into() }, INVALID_ARGUMENT)]
#[test_case(Error::NotFound { what: "profile `foo`".into() }, NOT_FOUND)]
#[test_case(Error::Conflict { reason: ConflictReason::NameTaken , detail: None }, CONFLICT)]
#[test_case(Error::InvalidTransition { from: "a".into(), to: "b".into() }, INVALID_TRANSITION)]
#[test_case(Error::LeaseHeld { client_id: ClientId::new() }, LEASE_HELD)]
#[test_case(Error::LeaseLost, LEASE_LOST)]
#[test_case(Error::CapExceeded { retry_after: Some(3), queue_pos: None }, CAP_EXCEEDED)]
#[test_case(Error::QuotaExceeded { scope: QuotaScope::DiskFloor, used_bytes: 1, limit_bytes: 2 }, QUOTA_EXCEEDED)]
#[test_case(Error::DownloadTooLarge { limit_bytes: 1 }, DOWNLOAD_TOO_LARGE)]
#[test_case(Error::RequestTooLarge { limit_bytes: 1 << 20 }, REQUEST_TOO_LARGE)]
#[test_case(Error::BuildMissing { reason: "r".into() }, BUILD_MISSING)]
#[test_case(Error::EngineUnsupported { capability: Capability::Pdf }, ENGINE_UNSUPPORTED)]
#[test_case(Error::DependencyMissing { name: "Xvfb".into(), help: "install it and retry".into() }, DEPENDENCY_MISSING)]
#[test_case(Error::ProxyUnhealthy, PROXY_UNHEALTHY)]
#[test_case(Error::NavigationFailed { net_error: "e".into() }, NAVIGATION_FAILED)]
#[test_case(Error::ElementNotFound, ELEMENT_NOT_FOUND)]
#[test_case(Error::StaleRef, STALE_REF)]
#[test_case(Error::EvaluationFailed { message: "m".into() }, EVALUATION_FAILED)]
#[test_case(Error::LoginFailed { step: 2, reason: LoginFailure::Rejected }, LOGIN_FAILED)]
#[test_case(Error::CaptchaBudgetExceeded, CAPTCHA_BUDGET_EXCEEDED)]
#[test_case(Error::CaptchaFailed, CAPTCHA_FAILED)]
#[test_case(Error::StorageUnavailable { message: "m".into() }, STORAGE_UNAVAILABLE)]
#[test_case(Error::Timeout { after_ms: 5 }, TIMEOUT)]
#[test_case(Error::BitwardenLocked, BITWARDEN_LOCKED)]
#[test_case(Error::Internal { message: "m".into() }, INTERNAL)]
fn error_maps_to_code_status_and_exit(error: Error, expected: Expected) {
    assert_eq!(
        (error.code(), error.http_status(), error.exit_code()),
        expected
    );
}

#[test_case(Error::LeaseHeld { client_id: ClientId::new() })]
#[test_case(Error::LeaseLost)]
#[test_case(Error::CapExceeded { retry_after: Some(3), queue_pos: Some(1) })]
#[test_case(Error::Conflict { reason: ConflictReason::Downgrade , detail: None })]
#[test_case(Error::Timeout { after_ms: 9 })]
#[test_case(Error::CaptchaFailed)]
#[test_case(Error::Conflict { reason: ConflictReason::ClientGone , detail: None })]
#[test_case(Error::Conflict { reason: ConflictReason::AttachDeclined , detail: None })]
#[test_case(Error::Conflict { reason: ConflictReason::AttachUnanswered , detail: None })]
#[test_case(Error::Conflict { reason: ConflictReason::ExtensionUnanswered , detail: None })]
fn body_round_trip(error: Error) {
    let body = error.body();
    let wire: ErrorBody = serde_json::from_value(serde_json::to_value(&body).unwrap()).unwrap();

    assert_eq!(wire.code, error.code());
    assert_eq!(wire.message, error.to_string());
    assert_eq!(Error::from_body(wire), error);
}

#[test]
fn body_details_hold_variant_fields() {
    let client_id = ClientId::new();
    let held = Error::LeaseHeld {
        client_id: client_id.clone(),
    };

    assert_eq!(
        held.body().details,
        json!({ "client_id": client_id.as_str() })
    );
    assert_eq!(Error::LeaseLost.body().details, json!({}));
}

#[test]
fn body_message_tells_what_to_do() {
    let error = Error::Conflict {
        reason: ConflictReason::ProfileLocked,
        detail: None,
    };

    assert_eq!(error.body().message, CONFLICT_PROFILE_LOCKED_MESSAGE);
}

#[test]
fn from_body_unknown_code_becomes_internal() {
    let body = ErrorBody {
        code: UNKNOWN_CODE.to_string(),
        message: "boom".to_string(),
        details: json!({ "x": 1 }),
    };

    assert_eq!(
        Error::from_body(body),
        Error::Internal {
            message: "boom".to_string()
        }
    );
}

#[test]
fn quota_scope_serializes_snake_case() {
    assert_eq!(
        Error::QuotaExceeded {
            scope: QuotaScope::DiskFloor,
            used_bytes: 1,
            limit_bytes: 2,
        }
        .body()
        .details,
        json!({ "scope": "disk_floor", "used_bytes": 1, "limit_bytes": 2 })
    );
}

#[test_case(ConflictReason::ClientGone, "client_gone")]
#[test_case(ConflictReason::ProfileNotRunning, "profile_not_running")]
#[test_case(ConflictReason::AttachDeclined, "attach_declined")]
#[test_case(ConflictReason::AttachUnanswered, "attach_unanswered")]
#[test_case(ConflictReason::ExtensionUnanswered, "extension_unanswered")]
#[test_case(ConflictReason::SecretOnPage, "secret_on_page")]
#[test_case(ConflictReason::CaptchaInProgress, "captcha_in_progress")]
#[test_case(ConflictReason::CredentialExists, "credential_exists")]
fn conflict_reason_wire_name(reason: ConflictReason, wire: &str) {
    let error = Error::Conflict {
        reason,
        detail: None,
    };
    let body = error.body();

    assert_eq!(body.code, "Conflict");
    assert_eq!(body.details, json!({ "reason": wire }));
    assert_eq!((error.http_status(), error.exit_code()), (409, 4));
}

#[test_case(
    ConflictReason::ProfileNotRunning,
    "profile is not running; launch it first"
)]
#[test_case(
    ConflictReason::AttachDeclined,
    r#"the browser declined the remote-debugging connection; attach again and click Allow in its "Allow remote debugging?" dialog"#
)]
#[test_case(
    ConflictReason::AttachUnanswered,
    r#"the browser did not answer within 30 s; click Allow in its "Allow remote debugging?" dialog and attach again"#
)]
#[test_case(
    ConflictReason::ExtensionUnanswered,
    "no companion connected within 30 s; click the brr icon in Chrome and choose Allow and connect"
)]
fn conflict_reason_message(reason: ConflictReason, message: &str) {
    assert_eq!(
        Error::Conflict {
            reason,
            detail: None
        }
        .body()
        .message,
        message
    );
}

#[test]
fn conflict_detail_round_trips_and_is_omitted_when_none() {
    let with = Error::Conflict {
        reason: ConflictReason::Referenced,
        detail: Some("profile pinner".to_string()),
    };
    let without = Error::Conflict {
        reason: ConflictReason::Referenced,
        detail: None,
    };

    assert_eq!(
        with.body().details,
        json!({ "reason": "referenced", "detail": "profile pinner" })
    );
    assert!(with.body().message.ends_with(" (profile pinner)"));
    assert_eq!(Error::from_body(with.body()), with);
    assert_eq!(without.body().details, json!({ "reason": "referenced" }));
    assert_eq!(Error::from_body(without.body()), without);
}

#[test]
fn request_too_large_message_names_the_limit_in_mib() {
    let error = Error::RequestTooLarge {
        limit_bytes: 66 << 20,
    };

    assert_eq!(
        error.body().message,
        "request body exceeds 66 MiB; send a smaller file"
    );
    assert_eq!(error.body().details, json!({ "limit_bytes": 66 << 20 }));
}

#[test]
fn login_failed_carries_its_reason() {
    let error = Error::LoginFailed {
        step: 2,
        reason: LoginFailure::NoTotpSecret,
    };
    let body = error.body();

    assert_eq!(
        body.details,
        json!({ "step": 2, "reason": "no_totp_secret" })
    );
    assert_eq!(
        body.message,
        "login failed at step 2: the site asks for a one-time code but the credential has no TOTP secret"
    );
    assert_eq!(Error::from_body(body), error);
}

#[test_case(QuotaScope::Profile, 4_200_000, 1_000_000, "profile data is 4.2 MB, over its 1 MB quota; delete data, or raise the quota under Profile → Settings → Storage"; "profile")]
#[test_case(QuotaScope::DiskFloor, 999, 5_000_000_000, "free disk is 999 B, below the 5 GB floor; free space or lower retention.free_disk_floor_gb"; "disk floor")]
fn quota_message_uses_human_sizes(scope: QuotaScope, used: u64, limit: u64, expected: &str) {
    let error = Error::QuotaExceeded {
        scope,
        used_bytes: used,
        limit_bytes: limit,
    };

    assert_eq!(error.to_string(), expected);
}

#[test_case("invalid value: integer `5000000000`, expected u32", Some("must be at most 4294967295"); "too large")]
#[test_case("invalid value: integer `-5`, expected u64", Some("must be at least 0"); "negative")]
#[test_case("invalid type: string \"a\", expected u32", None; "not a number")]
fn range_message_names_the_range(message: &str, expected: Option<&str>) {
    assert_eq!(range_message(message).as_deref(), expected);
}
