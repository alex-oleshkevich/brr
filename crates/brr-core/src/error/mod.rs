use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{id::ClientId, model::Capability};

pub const EXIT_DAEMON_UNREACHABLE: u8 = 69;

const BYTES_PER_MIB: u64 = 1 << 20;
const CODE_KEY: &str = "code";
const DETAILS_KEY: &str = "details";
// Decimal, like the GB a person types as a Profile's quota.
const SIZE_STEP: f64 = 1000.0;
const BITWARDEN_LOCKED_HELP: &str = "Bitwarden is not connected or its vault could not be opened; connect it in Settings → Logins, or check bitwarden.status for the last error";
const SIZE_UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
const SERDE_INTEGER: &str = "invalid value: integer `";
const SERDE_EXPECTED: &str = "`, expected ";

#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "code", content = "details")]
pub enum Error {
    #[error("invalid `{field}`: {message}")]
    InvalidArgument { field: String, message: String },
    #[error("{what} not found")]
    NotFound { what: String },
    #[error("{reason}{}", .detail.as_ref().map_or_else(String::new, |d| format!(" ({d})")))]
    Conflict {
        reason: ConflictReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    #[error("cannot go from {from} to {to}")]
    InvalidTransition { from: String, to: String },
    #[error(
        "tab is leased by client {client_id}; use a read-only tool or wait until its lease ends"
    )]
    LeaseHeld { client_id: ClientId },
    #[error("the lease was released during the call; acquire a new lease and retry")]
    LeaseLost,
    #[error("instance or tab limit reached; retry later or raise the limits")]
    CapExceeded {
        retry_after: Option<u64>,
        queue_pos: Option<u32>,
    },
    // For disk_floor, used_bytes is the free space and limit_bytes the floor it fell below.
    #[error("{}", quota_message(*scope, *used_bytes, *limit_bytes))]
    QuotaExceeded {
        scope: QuotaScope,
        used_bytes: u64,
        limit_bytes: u64,
    },
    #[error("download is larger than {limit_bytes} bytes; raise the download limit or skip it")]
    DownloadTooLarge { limit_bytes: u64 },
    #[error("request body exceeds {} MiB; send a smaller file", limit_bytes / BYTES_PER_MIB)]
    RequestTooLarge { limit_bytes: u64 },
    #[error("build missing: {reason}")]
    BuildMissing { reason: String },
    #[error("this browser version can't do {}; choose another browser version", capability.as_str().replace('_', " "))]
    EngineUnsupported { capability: Capability },
    #[error("{name} is not available; {help}")]
    DependencyMissing { name: String, help: String },
    #[error("the proxy failed its health check; pick another proxy or fix this one")]
    ProxyUnhealthy,
    #[error("navigation failed: {net_error}")]
    NavigationFailed { net_error: String },
    #[error("element not found; read the page again and pick a current element")]
    ElementNotFound,
    #[error("element ref is stale; take a new snapshot to get fresh refs")]
    StaleRef,
    #[error("script evaluation failed: {message}")]
    EvaluationFailed { message: String },
    #[error("login failed at step {step}: {reason}")]
    LoginFailed { step: u8, reason: LoginFailure },
    #[error("captcha budget is spent; raise captcha.daily_usd or wait until tomorrow")]
    CaptchaBudgetExceeded,
    #[error("captcha could not be solved; retry or solve it by hand")]
    CaptchaFailed,
    #[error("storage backend unavailable: {message}")]
    StorageUnavailable { message: String },
    #[error("timed out after {after_ms} ms; retry with a longer timeout")]
    Timeout { after_ms: u64 },
    #[error("internal error: {message}")]
    Internal { message: String },
    #[error("{BITWARDEN_LOCKED_HELP}")]
    BitwardenLocked,
}

// Fixed strings: a site's own error text may echo the username or password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoginFailure {
    NoForm,
    Rejected,
    NoTotpSecret,
    NotLoggedIn,
}

impl fmt::Display for LoginFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LoginFailure::NoForm => "no login form was found on the page; check the login URL",
            LoginFailure::Rejected => {
                "the site did not accept what was typed; check the credential"
            }
            LoginFailure::NoTotpSecret => {
                "the site asks for a one-time code but the credential has no TOTP secret"
            }
            LoginFailure::NotLoggedIn => {
                "the form went through but the site does not show a logged-in page"
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConflictReason {
    ProfileLocked,
    ProfileRunning,
    Downgrade,
    CredentialNotLinked,
    NameTaken,
    Builtin,
    Referenced,
    DaemonRunning,
    ClientGone,
    ProfileNotRunning,
    AttachDeclined,
    AttachUnanswered,
    ExtensionUnanswered,
    SecretOnPage,
    CaptchaInProgress,
    CredentialExists,
    TemplateGone,
}

impl fmt::Display for ConflictReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ConflictReason::ProfileLocked => {
                "profile is already running in another instance; stop it or use an ephemeral profile"
            }
            ConflictReason::ProfileRunning => "profile is running; stop its instance first",
            ConflictReason::Downgrade => {
                "build is older than the version that last used this profile; pass force to launch anyway"
            }
            ConflictReason::CredentialNotLinked => {
                "credential is not linked to this profile; link it first"
            }
            ConflictReason::NameTaken => "name is already taken; choose another name",
            ConflictReason::Builtin => "built-in items cannot be changed; copy it instead",
            ConflictReason::Referenced => {
                "it is still referenced by other items; remove those references first"
            }
            ConflictReason::DaemonRunning => "another brr daemon is running; stop it with `brr daemon stop`",
            ConflictReason::ClientGone => {
                "the Client that held this Tab has ended; the Tab stays with you"
            }
            ConflictReason::ProfileNotRunning => "profile is not running; launch it first",
            ConflictReason::AttachDeclined => {
                r#"the browser declined the remote-debugging connection; attach again and click Allow in its "Allow remote debugging?" dialog"#
            }
            ConflictReason::SecretOnPage => "the page holds a secret brr filled",
            ConflictReason::CaptchaInProgress => {
                "this Tab is already handling a captcha; wait for it to finish"
            }
            ConflictReason::CredentialExists => {
                "a credential for this site and username already exists; update it instead"
            }
            ConflictReason::ExtensionUnanswered => {
                "no companion connected within 30 s; click the brr icon in Chrome and choose Allow and connect"
            }
            ConflictReason::AttachUnanswered => {
                r#"the browser did not answer within 30 s; click Allow in its "Allow remote debugging?" dialog and attach again"#
            }
            ConflictReason::TemplateGone => {
                "the template this profile came from was deleted; there is nothing to re-apply"
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuotaScope {
    Profile,
    DiskFloor,
}

fn quota_message(scope: QuotaScope, used_bytes: u64, limit_bytes: u64) -> String {
    let (used, limit) = (human_size(used_bytes), human_size(limit_bytes));

    match scope {
        QuotaScope::Profile => format!(
            "profile data is {used}, over its {limit} quota; delete data, or raise the quota under Profile → Settings → Storage"
        ),
        QuotaScope::DiskFloor => format!(
            "free disk is {used}, below the {limit} floor; free space or lower retention.free_disk_floor_gb"
        ),
    }
}

fn human_size(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= SIZE_STEP && unit < SIZE_UNITS.len() - 1 {
        value /= SIZE_STEP;
        unit += 1;
    }
    let text = format!("{value:.1}");

    format!(
        "{} {}",
        text.strip_suffix(".0").unwrap_or(&text),
        SIZE_UNITS[unit]
    )
}

// serde words an out-of-range integer as "invalid value: integer `-5`, expected u32"; a person
// needs the range instead.
pub fn range_message(message: &str) -> Option<String> {
    let (value, expected) = message
        .trim()
        .strip_prefix(SERDE_INTEGER)?
        .split_once(SERDE_EXPECTED)?;
    let max = match expected {
        "u8" => u64::from(u8::MAX),
        "u16" => u64::from(u16::MAX),
        "u32" => u64::from(u32::MAX),
        "u64" | "usize" => u64::MAX,
        _ => return None,
    };

    Some(if value.starts_with('-') {
        "must be at least 0".to_owned()
    } else {
        format!("must be at most {max}")
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub details: Value,
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Error::InvalidArgument { .. } => "InvalidArgument",
            Error::NotFound { .. } => "NotFound",
            Error::Conflict { .. } => "Conflict",
            Error::InvalidTransition { .. } => "InvalidTransition",
            Error::LeaseHeld { .. } => "LeaseHeld",
            Error::LeaseLost => "LeaseLost",
            Error::CapExceeded { .. } => "CapExceeded",
            Error::QuotaExceeded { .. } => "QuotaExceeded",
            Error::DownloadTooLarge { .. } => "DownloadTooLarge",
            Error::RequestTooLarge { .. } => "RequestTooLarge",
            Error::BuildMissing { .. } => "BuildMissing",
            Error::EngineUnsupported { .. } => "EngineUnsupported",
            Error::DependencyMissing { .. } => "DependencyMissing",
            Error::ProxyUnhealthy => "ProxyUnhealthy",
            Error::NavigationFailed { .. } => "NavigationFailed",
            Error::ElementNotFound => "ElementNotFound",
            Error::StaleRef => "StaleRef",
            Error::EvaluationFailed { .. } => "EvaluationFailed",
            Error::LoginFailed { .. } => "LoginFailed",
            Error::CaptchaBudgetExceeded => "CaptchaBudgetExceeded",
            Error::CaptchaFailed => "CaptchaFailed",
            Error::StorageUnavailable { .. } => "StorageUnavailable",
            Error::Timeout { .. } => "Timeout",
            Error::BitwardenLocked => "BitwardenLocked",
            Error::Internal { .. } => "Internal",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            Error::InvalidArgument { .. } => 400,
            Error::NotFound { .. } => 404,
            Error::Conflict { .. } | Error::InvalidTransition { .. } | Error::BitwardenLocked => {
                409
            }
            Error::LeaseHeld { .. } => 423,
            Error::LeaseLost => 410,
            Error::CapExceeded { .. } => 429,
            Error::QuotaExceeded { .. } => 507,
            Error::DownloadTooLarge { .. } | Error::RequestTooLarge { .. } => 413,
            Error::BuildMissing { .. } => 424,
            Error::EngineUnsupported { .. }
            | Error::ElementNotFound
            | Error::StaleRef
            | Error::EvaluationFailed { .. }
            | Error::LoginFailed { .. }
            | Error::CaptchaFailed => 422,
            Error::DependencyMissing { .. } | Error::StorageUnavailable { .. } => 503,
            Error::ProxyUnhealthy | Error::NavigationFailed { .. } => 502,
            Error::CaptchaBudgetExceeded => 402,
            Error::Timeout { .. } => 504,
            Error::Internal { .. } => 500,
        }
    }

    pub fn exit_code(&self) -> u8 {
        match self {
            Error::Internal { .. } => 1,
            Error::InvalidArgument { .. } | Error::RequestTooLarge { .. } => 2,
            Error::NotFound { .. } => 3,
            Error::Conflict { .. } => 4,
            Error::InvalidTransition { .. } => 5,
            Error::LeaseHeld { .. } => 6,
            Error::LeaseLost => 7,
            Error::CapExceeded { .. } => 8,
            Error::QuotaExceeded { .. } => 9,
            Error::DownloadTooLarge { .. } => 10,
            Error::BuildMissing { .. } => 11,
            Error::EngineUnsupported { .. } => 12,
            Error::DependencyMissing { .. } => 13,
            Error::ProxyUnhealthy => 14,
            Error::NavigationFailed { .. } => 15,
            Error::ElementNotFound | Error::StaleRef => 16,
            Error::EvaluationFailed { .. } => 17,
            Error::LoginFailed { .. } => 18,
            Error::CaptchaBudgetExceeded => 19,
            Error::CaptchaFailed => 20,
            Error::StorageUnavailable { .. } => 21,
            Error::Timeout { .. } => 22,
            Error::BitwardenLocked => 23,
        }
    }

    // The wire form always carries `details`; serializing Error directly omits it for unit variants.
    pub fn body(&self) -> ErrorBody {
        let details = match serde_json::to_value(self) {
            Ok(Value::Object(mut tagged)) => tagged.remove(DETAILS_KEY),
            _ => None,
        };

        ErrorBody {
            code: self.code().to_string(),
            message: self.to_string(),
            details: details.unwrap_or_else(|| json!({})),
        }
    }

    pub fn from_body(body: ErrorBody) -> Error {
        let mut tagged = Map::new();
        tagged.insert(CODE_KEY.to_string(), Value::String(body.code));

        // Unit variants deserialize only when `details` is absent.
        if !matches!(&body.details, Value::Object(details) if details.is_empty()) {
            tagged.insert(DETAILS_KEY.to_string(), body.details);
        }

        serde_json::from_value(Value::Object(tagged)).unwrap_or(Error::Internal {
            message: body.message,
        })
    }
}

#[cfg(test)]
mod tests;
