use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    net::{Ipv4Addr, SocketAddr},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, Table, ser::ValueSerializer};

use crate::{
    AdmissionMode, BackendKind, CaptchaPolicy, Error, KeySource, LocalTime, SandboxMode, Theme,
    Toolset, UsdMicros, range_message,
};

#[cfg(test)]
mod tests;

const ENV_PREFIX: &str = "BRR_";
const ENV_SEPARATOR: &str = "__";
const KEY_SEPARATOR: char = '.';
const LIST_SEPARATOR: char = ',';
const DEFAULT_CONFIG_MODE: u32 = 0o600;
const MODE_MASK: u32 = 0o777;
const TEMP_EXTENSION: &str = "tmp";
const MAX_TIMEOUT_S: u64 = 3_600;
const MAX_WAIT_S: u64 = 86_400;
const MAX_FREE_RAM_MB: u64 = 1_048_576;
const MAX_BUFFER: u64 = 100_000;
const MIN_CPUS: f64 = 0.1;
const MAX_CPUS: f64 = 64.0;
const URL_SCHEMES: [&str; 2] = ["http://", "https://"];
const URL_MESSAGE: &str = "must be an http or https URL, for example https://example.com";
const VALUE_PROBE_KEY: &str = "value";
pub const STORAGE_SECRET: &str = "storage.secret_access_key";
const DEFAULT_PORT: u16 = 9898;
const DEFAULT_DAILY_USD_MICROS: u64 = 5_000_000;
const DEFAULT_BITWARDEN_URL: &str = "https://vault.bitwarden.com";
const LOCAL_HTTP_PREFIXES: [&str; 2] = ["http://localhost", "http://127.0.0.1"];
const BITWARDEN_URL_MESSAGE: &str =
    "must be an https URL, for example https://vault.bitwarden.com (http only for localhost)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Default,
    File,
    Env,
}

pub type Sources = BTreeMap<String, Source>;

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub server: ServerSettings,
    pub limits: LimitsSettings,
    pub adaptive: AdaptiveSettings,
    pub admission: AdmissionSettings,
    pub instance: InstanceSettings,
    pub idle: IdleSettings,
    pub lease: LeaseSettings,
    pub vault: VaultSettings,
    pub storage: StorageSettings,
    pub client: ClientSettings,
    pub mcp: McpSettings,
    pub oneshot: OneshotSettings,
    pub results: ResultsSettings,
    pub artifacts: ArtifactSettings,
    pub tab: TabSettings,
    pub login_check: LoginCheckSettings,
    pub proxy: ProxySettings,
    pub captcha: CaptchaSettings,
    pub content: ContentSettings,
    pub builds: BuildsSettings,
    pub extensions: ExtensionsSettings,
    pub backup: BackupSettings,
    pub retention: RetentionSettings,
    pub ui: UiSettings,
    pub sandbox: SandboxSettings,
    pub bitwarden: BitwardenSettings,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ServerSettings {
    pub bind: SocketAddr,
    pub socket: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LimitMode {
    Static,
    Adaptive,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LimitsSettings {
    pub mode: LimitMode,
    pub max_instances: u32,
    pub max_instances_per_engine: EngineLimits,
    pub max_instances_per_tag: BTreeMap<String, u32>,
    pub max_tabs_per_instance: u32,
    pub max_tabs_total: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct EngineLimits {
    pub chromium: u32,
    pub camoufox: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AdaptiveSettings {
    pub min_free_ram_mb: u64,
    pub max_load_per_core: f64,
    pub reserve_mb: ReserveMb,
    pub pressure_check_s: u64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ReserveMb {
    pub chromium_headless: u64,
    pub chromium_display: u64,
    pub camoufox: u64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AdmissionSettings {
    pub default: AdmissionMode,
    pub queue_timeout_s: u64,
    pub priority: u8,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct InstanceSettings {
    pub start_timeout_s: u64,
    pub stop_timeout_s: u64,
    pub auto_restart: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct IdleSettings {
    pub tab_min: u32,
    pub instance_min: u32,
    pub grace_s: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LeaseSettings {
    pub exclusive_wait_s: u64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct VaultSettings {
    pub key_source: Option<KeySource>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct StorageSettings {
    pub backend: BackendKind,
    pub endpoint: Option<String>,
    pub region: Option<String>,
    pub bucket: Option<String>,
    pub access_key_id: Option<String>,
    pub path_style: bool,
    pub r2_account_id: Option<String>,
    pub public_endpoint: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ClientSettings {
    pub rest_idle_min: u32,
    pub mcp_http_idle_min: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct McpSettings {
    pub default_toolsets: Vec<Toolset>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct OneshotSettings {
    pub profile: String,
    pub pool_size: u32,
    pub linger_s: u64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ResultsSettings {
    pub inline_max_mb: u64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ArtifactSettings {
    pub url_ttl_s: u64,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TabSettings {
    pub console_buffer: usize,
    pub network_buffer: usize,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct LoginCheckSettings {
    pub interval_min: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ProxySettings {
    pub health_interval_min: u32,
    pub check_url: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CaptchaSettings {
    pub daily_usd: UsdMicros,
    pub daily_solves: u32,
    pub human_wait_s: u64,
    pub default_policy: CaptchaPolicy,
    pub provider_url: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ContentSettings {
    pub adblock_lists: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BuildsSettings {
    pub check_at: LocalTime,
    pub github_api: String,
    pub edge_repo: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ExtensionsSettings {
    pub cws_update_url: String,
    pub amo_api: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BackupSettings {
    pub keep: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RetentionSettings {
    pub artifacts_max_age_days: u32,
    pub artifacts_max_total_gb: u64,
    pub download_max_mb: u64,
    pub free_disk_floor_gb: u64,
    pub unused_days: u32,
    pub events_days: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UiSettings {
    pub theme: Theme,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SandboxSettings {
    pub mode: SandboxMode,
    // None: the image of the container brr itself runs in.
    pub image: Option<String>,
    pub memory_mb: u32,
    pub cpus: f64,
    pub pids: u32,
    pub shm_mb: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BitwardenSettings {
    // https, or http only for localhost, so a vault password never crosses the network in clear.
    pub server_url: String,
    pub sync_interval_min: u32,
}

impl Default for ServerSettings {
    fn default() -> ServerSettings {
        ServerSettings {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT)),
            socket: None,
        }
    }
}

impl Default for LimitsSettings {
    fn default() -> LimitsSettings {
        LimitsSettings {
            mode: LimitMode::Static,
            max_instances: 8,
            max_instances_per_engine: EngineLimits::default(),
            max_instances_per_tag: BTreeMap::new(),
            max_tabs_per_instance: 20,
            max_tabs_total: 100,
        }
    }
}

impl Default for EngineLimits {
    fn default() -> EngineLimits {
        EngineLimits {
            chromium: 8,
            camoufox: 4,
        }
    }
}

impl Default for AdaptiveSettings {
    fn default() -> AdaptiveSettings {
        AdaptiveSettings {
            min_free_ram_mb: 1024,
            max_load_per_core: 0.85,
            reserve_mb: ReserveMb::default(),
            pressure_check_s: 5,
        }
    }
}

impl Default for ReserveMb {
    fn default() -> ReserveMb {
        ReserveMb {
            chromium_headless: 400,
            chromium_display: 600,
            camoufox: 600,
        }
    }
}

impl Default for AdmissionSettings {
    fn default() -> AdmissionSettings {
        AdmissionSettings {
            default: AdmissionMode::Queue,
            queue_timeout_s: 60,
            priority: 5,
        }
    }
}

impl Default for InstanceSettings {
    fn default() -> InstanceSettings {
        InstanceSettings {
            start_timeout_s: 30,
            stop_timeout_s: 10,
            auto_restart: false,
        }
    }
}

impl Default for IdleSettings {
    fn default() -> IdleSettings {
        IdleSettings {
            tab_min: 15,
            instance_min: 30,
            grace_s: 60,
        }
    }
}

impl Default for LeaseSettings {
    fn default() -> LeaseSettings {
        LeaseSettings {
            exclusive_wait_s: 60,
        }
    }
}

impl Default for ClientSettings {
    fn default() -> ClientSettings {
        ClientSettings {
            rest_idle_min: 10,
            mcp_http_idle_min: 10,
        }
    }
}

impl Default for McpSettings {
    fn default() -> McpSettings {
        McpSettings {
            default_toolsets: Toolset::ALL.to_vec(),
        }
    }
}

impl Default for OneshotSettings {
    fn default() -> OneshotSettings {
        OneshotSettings {
            profile: "oneshot".to_string(),
            pool_size: 0,
            linger_s: 60,
        }
    }
}

impl Default for ResultsSettings {
    fn default() -> ResultsSettings {
        ResultsSettings { inline_max_mb: 5 }
    }
}

impl Default for ArtifactSettings {
    fn default() -> ArtifactSettings {
        ArtifactSettings { url_ttl_s: 3600 }
    }
}

impl Default for TabSettings {
    fn default() -> TabSettings {
        TabSettings {
            console_buffer: 1000,
            network_buffer: 1000,
        }
    }
}

impl Default for LoginCheckSettings {
    fn default() -> LoginCheckSettings {
        LoginCheckSettings { interval_min: 60 }
    }
}

impl Default for ProxySettings {
    fn default() -> ProxySettings {
        ProxySettings {
            health_interval_min: 60,
            check_url: "https://www.cloudflare.com/cdn-cgi/trace".to_string(),
        }
    }
}

impl Default for CaptchaSettings {
    fn default() -> CaptchaSettings {
        CaptchaSettings {
            daily_usd: UsdMicros(DEFAULT_DAILY_USD_MICROS),
            daily_solves: 200,
            human_wait_s: 300,
            default_policy: CaptchaPolicy::Auto,
            provider_url: "https://api.2captcha.com".to_string(),
        }
    }
}

impl Default for ContentSettings {
    fn default() -> ContentSettings {
        ContentSettings {
            adblock_lists: vec![
                "https://easylist.to/easylist/easylist.txt".to_string(),
                "https://easylist.to/easylist/easyprivacy.txt".to_string(),
            ],
        }
    }
}

impl Default for BuildsSettings {
    fn default() -> BuildsSettings {
        BuildsSettings {
            check_at: LocalTime { hour: 4, minute: 0 },
            github_api: "https://api.github.com".to_string(),
            edge_repo: "https://packages.microsoft.com/repos/edge".to_string(),
        }
    }
}

impl Default for ExtensionsSettings {
    fn default() -> ExtensionsSettings {
        ExtensionsSettings {
            cws_update_url: "https://clients2.google.com/service/update2/crx".to_string(),
            amo_api: "https://addons.mozilla.org/api/v5".to_string(),
        }
    }
}

impl Default for BackupSettings {
    fn default() -> BackupSettings {
        BackupSettings { keep: 7 }
    }
}

impl Default for RetentionSettings {
    fn default() -> RetentionSettings {
        RetentionSettings {
            artifacts_max_age_days: 7,
            artifacts_max_total_gb: 10,
            download_max_mb: 2048,
            free_disk_floor_gb: 5,
            unused_days: 30,
            events_days: 30,
        }
    }
}

impl Default for UiSettings {
    fn default() -> UiSettings {
        UiSettings {
            theme: Theme::System,
        }
    }
}

impl Default for SandboxSettings {
    fn default() -> SandboxSettings {
        SandboxSettings {
            mode: SandboxMode::default(),
            image: None,
            memory_mb: 2048,
            cpus: 2.0,
            pids: 1024,
            shm_mb: 1024,
        }
    }
}

impl Default for BitwardenSettings {
    fn default() -> BitwardenSettings {
        BitwardenSettings {
            server_url: DEFAULT_BITWARDEN_URL.to_string(),
            sync_interval_min: 15,
        }
    }
}

impl Settings {
    // Spec A1 minimums: zero here would queue every launch or cancel every Download with no hint
    // why. The maximums catch a slip of the keyboard before it becomes a day-long wait.
    pub fn validate(&self) -> Result<(), Error> {
        let buffer = |value: usize| u64::try_from(value).unwrap_or(u64::MAX);
        let bounds = [
            (
                "limits.max_instances",
                self.limits.max_instances.into(),
                1,
                u64::MAX,
            ),
            (
                "limits.max_instances_per_engine.chromium",
                self.limits.max_instances_per_engine.chromium.into(),
                1,
                u64::MAX,
            ),
            (
                "limits.max_instances_per_engine.camoufox",
                self.limits.max_instances_per_engine.camoufox.into(),
                1,
                u64::MAX,
            ),
            (
                "limits.max_tabs_per_instance",
                self.limits.max_tabs_per_instance.into(),
                1,
                u64::MAX,
            ),
            ("idle.tab_min", self.idle.tab_min.into(), 1, u64::MAX),
            (
                "idle.instance_min",
                self.idle.instance_min.into(),
                1,
                u64::MAX,
            ),
            (
                "retention.download_max_mb",
                self.retention.download_max_mb,
                1,
                u64::MAX,
            ),
            (
                "instance.start_timeout_s",
                self.instance.start_timeout_s,
                1,
                MAX_TIMEOUT_S,
            ),
            (
                "instance.stop_timeout_s",
                self.instance.stop_timeout_s,
                1,
                MAX_TIMEOUT_S,
            ),
            (
                "adaptive.min_free_ram_mb",
                self.adaptive.min_free_ram_mb,
                0,
                MAX_FREE_RAM_MB,
            ),
            (
                "tab.console_buffer",
                buffer(self.tab.console_buffer),
                1,
                MAX_BUFFER,
            ),
            (
                "tab.network_buffer",
                buffer(self.tab.network_buffer),
                1,
                MAX_BUFFER,
            ),
            (
                "admission.queue_timeout_s",
                self.admission.queue_timeout_s,
                0,
                MAX_WAIT_S,
            ),
            (
                "lease.exclusive_wait_s",
                self.lease.exclusive_wait_s,
                0,
                MAX_WAIT_S,
            ),
            (
                "sandbox.memory_mb",
                self.sandbox.memory_mb.into(),
                256,
                65_536,
            ),
            ("sandbox.pids", self.sandbox.pids.into(), 64, 65_536),
            ("sandbox.shm_mb", self.sandbox.shm_mb.into(), 64, 16_384),
            (
                "bitwarden.sync_interval_min",
                self.bitwarden.sync_interval_min.into(),
                1,
                1440,
            ),
        ];

        for (field, value, min, max) in bounds {
            if value < min {
                return Err(invalid_argument(field, format!("must be at least {min}")));
            }
            if value > max {
                return Err(invalid_argument(field, format!("must be at most {max}")));
            }
        }
        let cpus = self.sandbox.cpus;
        if !(MIN_CPUS..=MAX_CPUS).contains(&cpus) {
            return Err(invalid_argument(
                "sandbox.cpus",
                format!("must be between {MIN_CPUS} and {MAX_CPUS}"),
            ));
        }
        if let Some(image) = &self.sandbox.image
            && (image.trim().is_empty()
                || image.chars().any(|c| c.is_whitespace() || c.is_control()))
        {
            return Err(invalid_argument(
                "sandbox.image",
                "name an image without spaces, for example brr-sandbox:latest".to_string(),
            ));
        }
        if self.mcp.default_toolsets.is_empty() {
            return Err(invalid_argument(
                "mcp.default_toolsets",
                "name at least one toolset".to_string(),
            ));
        }
        if self.oneshot.profile.trim().is_empty() {
            return Err(invalid_argument(
                "oneshot.profile",
                "name the Profile that one-shot calls use, for example oneshot".to_string(),
            ));
        }
        let bitwarden_url = self.bitwarden.server_url.as_str();
        let local_http = LOCAL_HTTP_PREFIXES.iter().any(|prefix| {
            bitwarden_url
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
        });
        let https = bitwarden_url
            .strip_prefix("https://")
            .is_some_and(|rest| !rest.is_empty());
        if !https && !local_http {
            return Err(invalid_argument(
                "bitwarden.server_url",
                BITWARDEN_URL_MESSAGE.to_string(),
            ));
        }
        if self
            .server
            .socket
            .as_ref()
            .is_some_and(|socket| !socket.is_absolute())
        {
            return Err(invalid_argument(
                "server.socket",
                "must be an absolute path, for example /run/user/1000/brr/brr.sock".to_string(),
            ));
        }

        let urls = [
            ("builds.github_api", &self.builds.github_api),
            ("extensions.cws_update_url", &self.extensions.cws_update_url),
            ("extensions.amo_api", &self.extensions.amo_api),
            ("proxy.check_url", &self.proxy.check_url),
            ("captcha.provider_url", &self.captcha.provider_url),
        ];
        let lists = self
            .content
            .adblock_lists
            .iter()
            .map(|list| ("content.adblock_lists", list));
        for (field, url) in urls.into_iter().chain(lists) {
            let is_http = URL_SCHEMES.iter().any(|scheme| {
                url.strip_prefix(scheme)
                    .is_some_and(|rest| !rest.is_empty())
            });
            if !is_http {
                return Err(invalid_argument(field, URL_MESSAGE.to_string()));
            }
        }

        Ok(())
    }
}

pub fn load(config_file: &Path, env: &[(String, String)]) -> Result<(Settings, Sources), Error> {
    let mut table = match fs::read_to_string(config_file) {
        Ok(text) => text.parse::<toml::Table>().map_err(|error| {
            invalid_argument(&config_file.display().to_string(), error.to_string())
        })?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => toml::Table::new(),
        Err(error) => return Err(io_error(config_file, error)),
    };

    let mut sources = default_sources();
    for (section, keys) in &table {
        if let Some(keys) = keys.as_table() {
            for key in keys.keys() {
                sources.insert(format!("{section}{KEY_SEPARATOR}{key}"), Source::File);
            }
        }
    }
    table
        .clone()
        .try_into::<Settings>()
        .map_err(|error| invalid_argument(&config_file.display().to_string(), error.to_string()))?;

    for (name, value) in env {
        let Some((section, key)) = env_setting(name).filter(|_| !value.is_empty()) else {
            continue;
        };
        let leaf = format!("{section}{KEY_SEPARATOR}{key}");
        if !sources.contains_key(&leaf) {
            return Err(invalid_argument(
                name,
                format!("unknown setting `{leaf}`; check the variable name"),
            ));
        }

        let section_table = table
            .entry(&section)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| invalid_argument(name, format!("`{section}` is not a table")))?;
        section_table.insert(key, parse_env_value(value));
        table
            .clone()
            .try_into::<Settings>()
            .map_err(|error| invalid_argument(name, error.to_string()))?;
        sources.insert(leaf, Source::Env);
    }

    let settings = table
        .try_into::<Settings>()
        .map_err(|error| invalid_argument(&config_file.display().to_string(), error.to_string()))?;
    settings.validate()?;

    Ok((settings, sources))
}

pub fn patch(
    config_file: &Path,
    changes: &BTreeMap<String, serde_json::Value>,
) -> Result<(), Error> {
    let text = match fs::read_to_string(config_file) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io_error(config_file, error)),
    };
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|error| invalid_argument(&config_file.display().to_string(), error.to_string()))?;

    for (leaf, json) in changes {
        let (section, key) = leaf
            .split_once(KEY_SEPARATOR)
            .ok_or_else(|| invalid_argument(leaf, "expected `section.key`".to_string()))?;
        if !default_sources().contains_key(leaf) {
            return Err(invalid_argument(leaf, "unknown setting".to_string()));
        }

        let section_table = document
            .entry(section)
            .or_insert_with(|| Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| invalid_argument(leaf, format!("`{section}` is not a table")))?;

        if json.is_null() {
            section_table.remove(key);
        } else {
            let value = list_from_text(section, key, json)
                .unwrap_or_else(|| json.clone())
                .serialize(ValueSerializer::new())
                .map_err(|error| invalid_argument(leaf, error.to_string()))?;
            section_table.insert(key, Item::Value(value));
        }

        // `message()` leaves out the TOML source snippet and line numbers, which mean nothing to an API caller.
        toml::from_str::<Settings>(&document.to_string())
            .map_err(|error| {
                let message = error.message();
                invalid_argument(
                    leaf,
                    range_message(message).unwrap_or_else(|| message.to_string()),
                )
            })?
            .validate()?;
    }

    let temp_file = config_file.with_extension(TEMP_EXTENSION);
    // The temp file inherits the live file's mode so a 0600 config.toml stays private.
    let mode = fs::metadata(config_file)
        .map(|meta| meta.permissions().mode() & MODE_MASK)
        .unwrap_or(DEFAULT_CONFIG_MODE);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&temp_file)
        .map_err(|error| io_error(&temp_file, error))?;
    file.write_all(document.to_string().as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(&temp_file, error))?;
    fs::rename(&temp_file, config_file).map_err(|error| io_error(config_file, error))
}

// `browser,instances` typed for a list setting means the list of those items.
fn list_from_text(section: &str, key: &str, json: &serde_json::Value) -> Option<serde_json::Value> {
    let text = json.as_str()?;
    let defaults = serde_json::to_value(Settings::default()).ok()?;

    defaults.get(section)?.get(key)?.is_array().then(|| {
        text.split(LIST_SEPARATOR)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(serde_json::Value::from)
            .collect()
    })
}

fn default_sources() -> Sources {
    let mut sources = Sources::new();
    // Serializing the defaults is the one place that knows every section.key, so the key list can't drift from the structs.
    if let Ok(serde_json::Value::Object(sections)) = serde_json::to_value(Settings::default()) {
        for (section, keys) in sections {
            if let serde_json::Value::Object(keys) = keys {
                for key in keys.keys() {
                    sources.insert(format!("{section}{KEY_SEPARATOR}{key}"), Source::Default);
                }
            }
        }
    }

    sources
}

fn env_setting(name: &str) -> Option<(String, String)> {
    let (section, key) = name.strip_prefix(ENV_PREFIX)?.split_once(ENV_SEPARATOR)?;

    Some((section.to_lowercase(), key.to_lowercase()))
}

fn parse_env_value(raw: &str) -> toml::Value {
    toml::from_str::<toml::Table>(&format!("{VALUE_PROBE_KEY} = {raw}"))
        .ok()
        .and_then(|mut table| table.remove(VALUE_PROBE_KEY))
        .unwrap_or_else(|| toml::Value::String(raw.to_string()))
}

fn invalid_argument(field: &str, message: String) -> Error {
    Error::InvalidArgument {
        field: field.to_string(),
        message,
    }
}

fn io_error(path: &Path, error: io::Error) -> Error {
    Error::Internal {
        message: format!("{}: {error}", path.display()),
    }
}
