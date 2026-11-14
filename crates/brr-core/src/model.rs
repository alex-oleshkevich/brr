use std::{cmp::Ordering, collections::BTreeMap, ffi::OsString, fmt, path::PathBuf, str::FromStr};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::{
    error::Error,
    id::{
        ArtifactId, AssetId, BackupId, BuildId, CaptchaSolveId, ClientId, CredentialId,
        ExtensionId, InstanceId, LeaseId, LoginCheckId, PoolId, ProfileId, ProxyId, TabId,
        TargetId, TemplateId,
    },
    state::{InstanceState, TabState},
};

#[cfg(test)]
mod tests;

const DEFAULT_PAGE_LIMIT: u32 = 100;
const MAX_PAGE_LIMIT: u32 = 1000;
pub const MICROS_PER_USD: u64 = 1_000_000;
// Chromium: the pipe is the only control channel and the data dir is the Profile's. Firefox and
// Camoufox: `-profile` would point at another Profile's directory, the rest open a second channel.
const FORBIDDEN_SWITCH_PREFIXES: [&str; 5] = [
    "remote-debugging",
    "user-data-dir",
    "marionette",
    "start-debugger-server",
    "juggler",
];
// Exact names: a prefix would also block harmless switches like Chromium's `profile-directory`.
const FORBIDDEN_SWITCH_NAMES: [&str; 3] = ["profile", "new-instance", "no-remote"];
const FORBIDDEN_PREF_PREFIXES: [&str; 3] = ["marionette.", "devtools.debugger.remote", "remote."];
const FORBIDDEN_PREF_NAMES: [&str; 1] = ["devtools.chrome.enabled"];
const ARGS_FIELD: &str = "args";
const ENV_FIELD: &str = "env";
const CHANNEL_FIELD: &str = "channel";
const TOOLSETS_FIELD: &str = "toolsets";
const DEFAULT_CHANNEL_NAME: &str = "stable";
const RESERVED_ENV_PREFIX: &str = "BRR_";
const GEOLOCATION_FIELD: &str = "settings.geolocation";
const PREFS_FIELD: &str = "settings.prefs";
const VIEWPORT_FIELD: &str = "settings.viewport";
const FINGERPRINT_SCREEN_FIELD: &str = "settings.fingerprint.screen";
const MAX_PREF_KEY_LEN: usize = 200;
pub(crate) const MAX_SCREEN_SIDE: u32 = 10_000;

macro_rules! enums {
    ($($name:ident { $($variant:ident),+ $(,)? })+) => {$(
        #[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }
    )+};
}

enums! {
    Engine { Chromium, Camoufox }
    Capability { Pdf, Mhtml, Screencast, UnpackedExtensions, Har, RequestInterception, FingerprintConfig }
    BuildSource { Cft, HeadlessShell, Ungoogled, FingerprintChromium, Brave, Edge, System, Camoufox }
    BuildState { Installing, Installed, Failed, Registered }
    InstallPhase { Downloading, Extracting }
    Display { Headless, Visible, Virtual }
    ProfileKind { Persistent, Ephemeral }
    InstanceKind { Managed, Live }
    AdmissionMode { Queue, Reject, Evict }
    ClientKind { Mcp, Cli, Rest, Ui, Connect }
    WaitUntil { Load, Domcontentloaded, Networkidle }
    Toolset { Browser, Instances, Profiles, Builds, Network, Credentials, Captcha, Storage, System }
    Order { Asc, Desc }
    KeySource { Keyring, File }
    Theme { System, Light, Dark }
    ReleaseReason { Released, ClientEnded, TabClosed, InstanceStopped, InstanceCrashed, DaemonRestart, Transferred }
    ClientEndReason { Ended, Idle, Disconnected, DaemonRestart }
    EndReason { Stop, Shutdown, LastTabClosed, Cancel, QueueTimeout, LaunchError, StartTimeout, Crashed, DaemonRestart, Evicted, MemoryPressure, Reaped }
    TabCloseReason { Client, Page, InstanceStopped, Reaped, DaemonRestart }
    ArtifactKind { Download, Screenshot, Pdf, Mhtml, Har, Html, Markdown }
    LoginStatus { Unknown, Ok, Failed }
    HumanReason { Captcha, Dialog, Login }
    AdmissionOutcome { Admitted, Queued, Rejected, Evicted }
    FingerprintOs { Windows, Macos, Linux }
    CaptchaPolicy { Off, OnDemand, Auto, Human }
    ProxyScheme { Http, Https, Socks5 }
    PoolMode { Sticky, Rotate }
    ExtensionSource { Cws, Amo, Local }
    AssetKind { GeoipDb, AdblockList, AutoconsentRules, DeviceList, FpgenModel, FontsBundle }
    BuildRefKind { Profile, Channel, Instance }
    CookieFormat { Json, Netscape }
    PasswordCsvFormat { Chrome, Firefox }
    CaptchaType { RecaptchaV2, RecaptchaV3, Hcaptcha, Turnstile, Funcaptcha, Geetest, Image }
    CaptchaResult { Solved, Failed }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebRtcPolicy {
    #[default]
    Default,
    DefaultPublicAndPrivateInterfaces,
    DefaultPublicInterfaceOnly,
    DisableNonProxiedUdp,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Pdf => "pdf",
            Capability::Mhtml => "mhtml",
            Capability::Screencast => "screencast",
            Capability::UnpackedExtensions => "unpacked_extensions",
            Capability::Har => "har",
            Capability::RequestInterception => "request_interception",
            Capability::FingerprintConfig => "fingerprint_config",
        }
    }
}

impl BuildSource {
    const ALL: [BuildSource; 8] = [
        BuildSource::Cft,
        BuildSource::HeadlessShell,
        BuildSource::Ungoogled,
        BuildSource::FingerprintChromium,
        BuildSource::Brave,
        BuildSource::Edge,
        BuildSource::System,
        BuildSource::Camoufox,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BuildSource::Cft => "cft",
            BuildSource::HeadlessShell => "headless_shell",
            BuildSource::Ungoogled => "ungoogled",
            BuildSource::FingerprintChromium => "fingerprint_chromium",
            BuildSource::Brave => "brave",
            BuildSource::Edge => "edge",
            BuildSource::System => "system",
            BuildSource::Camoufox => "camoufox",
        }
    }
}

impl fmt::Display for BuildSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl BuildRefKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BuildRefKind::Profile => "profile",
            BuildRefKind::Channel => "channel",
            BuildRefKind::Instance => "instance",
        }
    }
}

impl fmt::Display for BuildRefKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for BuildSource {
    type Err = Error;

    fn from_str(s: &str) -> Result<BuildSource, Error> {
        BuildSource::ALL
            .into_iter()
            .find(|source| source.as_str() == s)
            .ok_or_else(|| Error::InvalidArgument {
                field: CHANNEL_FIELD.to_string(),
                message: format!(
                    "unknown build source; use one of: {}",
                    BuildSource::ALL.map(BuildSource::as_str).join(", ")
                ),
            })
    }
}

impl Toolset {
    pub const ALL: [Toolset; 9] = [
        Toolset::Browser,
        Toolset::Instances,
        Toolset::Profiles,
        Toolset::Builds,
        Toolset::Network,
        Toolset::Credentials,
        Toolset::Captcha,
        Toolset::Storage,
        Toolset::System,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Toolset::Browser => "browser",
            Toolset::Instances => "instances",
            Toolset::Profiles => "profiles",
            Toolset::Builds => "builds",
            Toolset::Network => "network",
            Toolset::Credentials => "credentials",
            Toolset::Captcha => "captcha",
            Toolset::Storage => "storage",
            Toolset::System => "system",
        }
    }

    pub fn parse_list(s: &str) -> Result<Vec<Toolset>, Error> {
        s.split(',')
            .map(str::trim)
            .map(|name| {
                Toolset::ALL
                    .into_iter()
                    .find(|toolset| toolset.as_str() == name)
                    .ok_or_else(|| Error::InvalidArgument {
                        field: TOOLSETS_FIELD.to_string(),
                        message: format!(
                            "unknown toolset; use any of: {}",
                            Toolset::ALL.map(Toolset::as_str).join(", ")
                        ),
                    })
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Channel {
    pub source: BuildSource,
    pub name: String,
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.source, self.name)
    }
}

impl FromStr for Channel {
    type Err = Error;

    fn from_str(s: &str) -> Result<Channel, Error> {
        let invalid = || Error::InvalidArgument {
            field: CHANNEL_FIELD.to_string(),
            message: "expected `<source>:<name>`, for example `cft:stable`".to_string(),
        };
        let (source, name) = s.split_once(':').ok_or_else(invalid)?;

        if name.is_empty() {
            return Err(invalid());
        }

        Ok(Channel {
            source: source.parse()?,
            name: name.to_string(),
        })
    }
}

impl Serialize for Channel {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Channel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Channel, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

impl JsonSchema for Channel {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Channel".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": "^[a-z_]+:.+$" })
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuildChoice {
    Channel(Channel),
    BuildId(BuildId),
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProxyChoice {
    ProxyId(ProxyId),
    PoolId(PoolId),
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Geolocation {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: Option<f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WebGl {
    pub vendor: String,
    pub renderer: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    #[serde(default)]
    pub os: Option<FingerprintOs>,
    #[serde(default)]
    pub screen: Option<Size>,
    #[serde(default)]
    pub fonts: Vec<String>,
    #[serde(default)]
    pub webgl: Option<WebGl>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct RetentionOverrides {
    pub artifacts_max_age_days: Option<u32>,
    pub artifacts_max_total_gb: Option<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsdMicros(pub u64);

impl Serialize for UsdMicros {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(self.0 as f64 / MICROS_PER_USD as f64)
    }
}

impl<'de> Deserialize<'de> for UsdMicros {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<UsdMicros, D::Error> {
        struct UsdVisitor;

        impl de::Visitor<'_> for UsdVisitor {
            type Value = UsdMicros;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a non-negative USD amount such as 5 or 5.5")
            }

            fn visit_u64<E: de::Error>(self, usd: u64) -> Result<UsdMicros, E> {
                usd.checked_mul(MICROS_PER_USD)
                    .map(UsdMicros)
                    .ok_or_else(|| E::custom("USD amount is too large"))
            }

            fn visit_i64<E: de::Error>(self, usd: i64) -> Result<UsdMicros, E> {
                u64::try_from(usd)
                    .map_err(|_| E::custom("USD amount must not be negative"))
                    .and_then(|usd| self.visit_u64(usd))
            }

            fn visit_f64<E: de::Error>(self, usd: f64) -> Result<UsdMicros, E> {
                if !usd.is_finite() || usd < 0.0 || usd > u64::MAX as f64 / MICROS_PER_USD as f64 {
                    return Err(E::custom("USD amount must be a non-negative number"));
                }

                Ok(UsdMicros((usd * MICROS_PER_USD as f64).round() as u64))
            }
        }

        deserializer.deserialize_any(UsdVisitor)
    }
}

impl JsonSchema for UsdMicros {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "UsdMicros".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "number" })
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub limit: u32,
    pub cursor: Option<String>,
}

impl Page {
    pub fn new(limit: Option<u32>, cursor: Option<String>) -> Page {
        Page {
            limit: limit.unwrap_or(DEFAULT_PAGE_LIMIT).clamp(1, MAX_PAGE_LIMIT),
            cursor,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct IdleOverrides {
    pub tab_min: Option<u32>,
    pub instance_min: Option<u32>,
    pub grace_s: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProfileSettings {
    pub build: BuildChoice,
    pub display: Display,
    #[serde(default)]
    pub screen: Option<Size>,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub idle: IdleOverrides,
    #[serde(default)]
    pub proxy: Option<ProxyChoice>,
    #[serde(default)]
    pub webrtc: WebRtcPolicy,
    #[serde(default)]
    pub prefs: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub geoip: bool,
    #[serde(default)]
    pub viewport: Option<Size>,
    #[serde(default)]
    pub geolocation: Option<Geolocation>,
    #[serde(default)]
    pub fingerprint: Option<Fingerprint>,
    #[serde(default)]
    pub captcha: Option<CaptchaPolicy>,
    #[serde(default)]
    pub block_ads: bool,
    #[serde(default)]
    pub dismiss_cookies: bool,
    #[serde(default)]
    pub companion: bool,
    #[serde(default)]
    pub download_max_mb: Option<u64>,
    #[serde(default)]
    pub retention: RetentionOverrides,
}

impl Default for ProfileSettings {
    fn default() -> ProfileSettings {
        ProfileSettings {
            build: BuildChoice::Channel(Channel {
                source: BuildSource::Cft,
                name: DEFAULT_CHANNEL_NAME.to_string(),
            }),
            display: Display::Headless,
            screen: None,
            locale: None,
            timezone: None,
            user_agent: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            idle: IdleOverrides::default(),
            proxy: None,
            webrtc: WebRtcPolicy::default(),
            prefs: BTreeMap::new(),
            geoip: false,
            viewport: None,
            geolocation: None,
            fingerprint: None,
            captcha: None,
            block_ads: false,
            dismiss_cookies: false,
            companion: false,
            download_max_mb: None,
            retention: RetentionOverrides::default(),
        }
    }
}

impl ProfileSettings {
    // Both guards protect process supervision: S1 kills by the BRR_INSTANCE_ID marker and brr
    // owns the only control connection to the browser. Browsers accept `-switch` as well as
    // `--switch`, so the dashes are ignored. The offending text is hostile, so it is not echoed.
    pub fn validate(&self) -> Result<(), Error> {
        if self.args.iter().any(|arg| {
            let switch = arg.trim_start_matches('-');
            let name = switch.split_once('=').map_or(switch, |(name, _)| name);

            FORBIDDEN_SWITCH_NAMES.contains(&name)
                || FORBIDDEN_SWITCH_PREFIXES
                    .iter()
                    .any(|prefix| switch.starts_with(prefix))
        }) {
            return Err(Error::InvalidArgument {
                field: ARGS_FIELD.to_string(),
                message: format!(
                    "switches {} are managed by brr; remove them",
                    FORBIDDEN_SWITCH_PREFIXES
                        .iter()
                        .chain(&FORBIDDEN_SWITCH_NAMES)
                        .copied()
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }

        if self
            .env
            .keys()
            .any(|key| key.starts_with(RESERVED_ENV_PREFIX))
        {
            return Err(Error::InvalidArgument {
                field: ENV_FIELD.to_string(),
                message: format!(
                    "variables starting with {RESERVED_ENV_PREFIX} are set by brr; remove them"
                ),
            });
        }

        // 0 would make a browser idle the moment it is ready, mid-navigation.
        for (field, minutes) in [
            ("idle.tab_min", self.idle.tab_min),
            ("idle.instance_min", self.idle.instance_min),
        ] {
            if minutes == Some(0) {
                return Err(Error::InvalidArgument {
                    field: field.to_string(),
                    message: "must be at least 1".to_string(),
                });
            }
        }

        if let Some(geolocation) = &self.geolocation
            && !(geolocation.accuracy.is_none_or(|accuracy| accuracy >= 0.0)
                && (-90.0..=90.0).contains(&geolocation.latitude)
                && (-180.0..=180.0).contains(&geolocation.longitude))
        {
            return Err(Error::InvalidArgument {
                field: GEOLOCATION_FIELD.to_string(),
                message: "latitude must be within -90..=90, longitude within -180..=180 and accuracy not negative".to_string(),
            });
        }

        if self
            .prefs
            .keys()
            .any(|key| key.is_empty() || key.len() > MAX_PREF_KEY_LEN)
        {
            return Err(Error::InvalidArgument {
                field: PREFS_FIELD.to_string(),
                message: format!("preference names must be 1..={MAX_PREF_KEY_LEN} characters"),
            });
        }

        if self.prefs.keys().any(|key| {
            FORBIDDEN_PREF_NAMES.contains(&key.as_str())
                || FORBIDDEN_PREF_PREFIXES
                    .iter()
                    .any(|prefix| key.starts_with(prefix))
        }) {
            return Err(Error::InvalidArgument {
                field: PREFS_FIELD.to_string(),
                message: format!(
                    "preferences {} are managed by brr; remove them",
                    FORBIDDEN_PREF_PREFIXES
                        .iter()
                        .chain(&FORBIDDEN_PREF_NAMES)
                        .copied()
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }

        if let Some(viewport) = &self.viewport {
            check_screen(viewport, VIEWPORT_FIELD)?;
        }

        if let Some(screen) = self.fingerprint.as_ref().and_then(|f| f.screen.as_ref()) {
            check_screen(screen, FINGERPRINT_SCREEN_FIELD)?;
        }

        Ok(())
    }
}

fn check_screen(size: &Size, field: &str) -> Result<(), Error> {
    if (1..=MAX_SCREEN_SIDE).contains(&size.width) && (1..=MAX_SCREEN_SIDE).contains(&size.height) {
        return Ok(());
    }

    Err(Error::InvalidArgument {
        field: field.to_string(),
        message: format!("width and height must be within 1..={MAX_SCREEN_SIDE}"),
    })
}

pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let part = |part: &str| part.parse::<u64>().unwrap_or(0);
    let mut a = a.split('.').map(part);
    let mut b = b.split('.').map(part);

    loop {
        match (a.next(), b.next()) {
            (None, None) => return Ordering::Equal,
            (x, y) => match x.unwrap_or(0).cmp(&y.unwrap_or(0)) {
                Ordering::Equal => {}
                unequal => return unequal,
            },
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Build {
    pub id: BuildId,
    pub source: BuildSource,
    pub engine: Engine,
    pub name: String,
    pub version: String,
    pub channel: Option<String>,
    pub path: PathBuf,
    pub state: BuildState,
    pub sha256: Option<String>,
    pub size_bytes: u64,
    pub capabilities: Vec<Capability>,
    pub installed_at: Option<i64>,
    pub last_referenced_at: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct BuildRef {
    pub build_id: BuildId,
    pub source: BuildSource,
    pub version: String,
    pub label: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct BuildReference {
    pub kind: BuildRefKind,
    pub id: String,
    pub label: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Proxy {
    pub id: ProxyId,
    pub scheme: ProxyScheme,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<SecretStatus>,
    pub healthy: Option<bool>,
    pub exit_ip: Option<String>,
    pub country: Option<String>,
    pub latency_ms: Option<u64>,
    // One plain sentence from the last failed check; None while healthy or unchecked.
    #[serde(default)]
    pub last_error: Option<String>,
    pub checked_at: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct Pool {
    pub id: PoolId,
    pub name: String,
    pub mode: PoolMode,
    pub proxy_ids: Vec<ProxyId>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct ExtensionVersion {
    pub version: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub fetched_at: i64,
    pub last_referenced_at: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    pub id: ExtensionId,
    pub engine: Engine,
    pub source: ExtensionSource,
    pub source_ref: String,
    pub name: String,
    pub versions: Vec<ExtensionVersion>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct ProfileExtension {
    pub extension_id: ExtensionId,
    pub pinned_version: Option<String>,
    pub auto_update: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub id: AssetId,
    pub kind: AssetKind,
    pub version: String,
    pub path: PathBuf,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    #[default]
    Local,
    S3,
    R2,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Local => "local",
            BackendKind::S3 => "s3",
            BackendKind::R2 => "r2",
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub id: ArtifactId,
    pub public_id: String,
    pub kind: ArtifactKind,
    pub profile_id: Option<ProfileId>,
    pub instance_id: Option<InstanceId>,
    pub tab_id: Option<TabId>,
    pub mime: String,
    pub size_bytes: u64,
    pub backend: BackendKind,
    pub location: String,
    pub sha256: String,
    pub created_at: i64,
    #[serde(default)]
    pub filename: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Template {
    pub id: TemplateId,
    pub name: String,
    pub builtin: bool,
    // One plain line of purpose: written for built-ins, the source Profile's notes for saved ones.
    #[serde(default)]
    pub description: Option<String>,
    pub settings: ProfileSettings,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub kind: ProfileKind,
    pub template_id: Option<TemplateId>,
    pub settings: ProfileSettings,
    pub pinned: bool,
    pub notes: String,
    pub tags: Vec<String>,
    pub data_dir: Option<PathBuf>,
    pub disk_quota_bytes: Option<u64>,
    pub backup_schedule: Option<String>,
    pub backup_keep: u32,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub login_status: Option<LoginStatus>,
    // Computed in the view: the Profile `oneshot.profile` names, which cannot be deleted.
    #[serde(default)]
    pub builtin: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Instance {
    pub id: InstanceId,
    pub kind: InstanceKind,
    pub profile_id: Option<ProfileId>,
    pub build_id: Option<BuildId>,
    pub state: InstanceState,
    pub display: Display,
    pub priority: u8,
    pub admission: AdmissionMode,
    pub pid: Option<u32>,
    pub log_path: Option<PathBuf>,
    pub cdp_url: Option<String>,
    pub data_dir: Option<PathBuf>,
    pub ephemeral: bool,
    pub promote_to_name: Option<String>,
    pub restart_count: u32,
    pub last_activity_at: i64,
    pub queued_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub end_reason: Option<EndReason>,
    pub error: Option<String>,
    #[serde(default)]
    pub profile_name: Option<String>,
    #[serde(default)]
    pub build: Option<BuildRef>,
    #[serde(default)]
    pub tab_count: u32,
    #[serde(default)]
    pub lease_count: u32,
    #[serde(default)]
    pub rss_bytes: Option<u64>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: TabId,
    pub instance_id: InstanceId,
    pub target_id: TargetId,
    pub state: TabState,
    pub url: String,
    pub title: String,
    pub opened_by_brr: bool,
    pub pinned: bool,
    pub last_activity_at: i64,
    pub created_at: i64,
    pub closed_at: Option<i64>,
    #[serde(default)]
    pub lease: Option<TabLease>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct TabLease {
    pub lease_id: LeaseId,
    pub client_id: ClientId,
    pub client_kind: ClientKind,
    pub client_label: String,
    pub transferred_from_client_id: Option<ClientId>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Client {
    pub id: ClientId,
    pub kind: ClientKind,
    pub label: String,
    pub current_tab_id: Option<TabId>,
    pub created_at: i64,
    pub ended_at: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Lease {
    pub id: LeaseId,
    pub client_id: ClientId,
    pub client_kind: ClientKind,
    pub client_label: String,
    pub instance_id: InstanceId,
    pub tab_id: Option<TabId>,
    pub exclusive: bool,
    pub acquired_at: i64,
    pub released_at: Option<i64>,
    pub release_reason: Option<ReleaseReason>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct SecretStatus {
    pub set: bool,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DisplayEnv {
    pub display: Option<OsString>,
    pub wayland_display: Option<OsString>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct CredentialSelectors {
    #[serde(default)]
    pub username_selector: Option<String>,
    #[serde(default)]
    pub password_selector: Option<String>,
    #[serde(default)]
    pub submit_selector: Option<String>,
    #[serde(default)]
    pub totp_selector: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct LastLogin {
    pub ok: bool,
    pub at: i64,
}

// Password, TOTP and notes are deliberately absent: no output type may carry them.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Credential {
    pub id: CredentialId,
    pub site: String,
    pub login_url: Option<String>,
    pub username: String,
    pub selectors: CredentialSelectors,
    pub totp_set: bool,
    pub profile_ids: Vec<ProfileId>,
    pub last_login: Option<LastLogin>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct LoginCheck {
    pub id: LoginCheckId,
    pub profile_id: ProfileId,
    pub site: String,
    pub url: String,
    pub expect_selector: Option<String>,
    pub expect_text: Option<String>,
    pub credential_id: Option<CredentialId>,
    pub auto_relogin: bool,
    pub last_status: LoginStatus,
    pub last_checked_at: Option<i64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct CaptchaSolve {
    pub id: CaptchaSolveId,
    pub ts: i64,
    pub profile_id: Option<ProfileId>,
    pub tab_id: Option<TabId>,
    pub site: String,
    #[serde(rename = "type")]
    pub captcha_type: CaptchaType,
    pub provider: String,
    pub cost_usd_micros: u64,
    pub duration_ms: u64,
    pub result: CaptchaResult,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Backup {
    pub id: BackupId,
    pub profile_id: ProfileId,
    // The Profile's name when the backup was made; kept after the Profile is deleted.
    pub profile_name: String,
    // Computed in the view: the Profile no longer exists, so the backup can only be restored or deleted.
    #[serde(default)]
    pub profile_deleted: bool,
    pub backend: String,
    pub location: String,
    pub size_bytes: u64,
    pub created_at: i64,
}

pub fn site_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();

    if scheme != "http" && scheme != "https" {
        return None;
    }

    // Browsers treat a backslash as a path separator in http(s) URLs.
    let authority = rest
        .find(['/', '\\', '?', '#'])
        .map_or(rest, |end| &rest[..end]);
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host_port = host_port.to_ascii_lowercase();
    let default_port = if scheme == "http" { ":80" } else { ":443" };
    let host_port = host_port.strip_suffix(default_port).unwrap_or(&host_port);

    if host_port.is_empty() {
        return None;
    }

    Some(format!("{scheme}://{host_port}"))
}
