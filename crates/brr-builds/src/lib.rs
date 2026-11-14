mod assets;
mod camoufox;
mod cft;
mod download;
mod edge;
mod extract;
mod github;
mod system;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

use std::{
    fmt::Display,
    fs,
    path::{Path, PathBuf},
};

use brr_core::{BuildSource, Capability, Error, InstallPhase, UtcDateTime};
use reqwest::StatusCode;
use schemars::JsonSchema;
use serde::{Serialize, de::DeserializeOwned};
use tokio::{sync::mpsc, task};

pub use assets::{
    AssetRelease, GEOIP_URL, fonts_bundle_release, install_fonts_bundle, install_geoip,
};
pub use camoufox::{
    CAMOUFOX_MIN_VERSION, CAMOUFOX_REPO, CompatRecord, camoufox_version_cmp, read_compat,
    write_compat,
};
pub use download::{Downloaded, Progress, download};
pub use extract::{ArchiveKind, extract};
pub use system::{Detected, detect_system};

pub const CFT_BASE_URL: &str = "https://googlechromelabs.github.io/chrome-for-testing";
pub const GITHUB_API_URL: &str = "https://api.github.com";
pub const EDGE_REPO_URL: &str = "https://packages.microsoft.com/repos/edge";

const MAX_JSON_BYTES: usize = 32 << 20;
const RATE_LIMIT_REMAINING: &str = "x-ratelimit-remaining";
const RATE_LIMIT_RESET: &str = "x-ratelimit-reset";
const USER_AGENT: &str = concat!("brr/", env!("CARGO_PKG_VERSION"));
const CHROMIUM_CHANNELS: [&str; 4] = ["stable", "beta", "dev", "canary"];
const STABLE_ONLY: [&str; 1] = ["stable"];
const LATEST_COMPATIBLE: [&str; 1] = ["latest-compatible"];
const CHROMIUM_CAPABILITIES: [Capability; 6] = [
    Capability::Pdf,
    Capability::Mhtml,
    Capability::Screencast,
    Capability::UnpackedExtensions,
    Capability::Har,
    Capability::RequestInterception,
];

#[derive(Debug, Clone, PartialEq)]
pub enum Want {
    Channel(String),
    Version(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceUrls {
    pub cft: String,
    pub github_api: String,
    pub edge_repo: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub source: BuildSource,
    pub version: String,
    pub channel: Option<String>,
    pub url: String,
    pub sha256: Option<String>,
}

#[derive(Serialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct Available {
    pub source: BuildSource,
    pub channel: Option<String>,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub executable: PathBuf,
    pub sha256: Option<String>,
    pub size_bytes: Option<u64>,
}

pub fn channels(source: BuildSource) -> &'static [&'static str] {
    match source {
        BuildSource::Cft | BuildSource::HeadlessShell => &CHROMIUM_CHANNELS,
        BuildSource::Ungoogled
        | BuildSource::FingerprintChromium
        | BuildSource::Brave
        | BuildSource::Edge => &STABLE_ONLY,
        BuildSource::Camoufox => &LATEST_COMPATIBLE,
        BuildSource::System => &[],
    }
}

pub fn capabilities(source: BuildSource) -> Vec<Capability> {
    match source {
        BuildSource::HeadlessShell => CHROMIUM_CAPABILITIES
            .into_iter()
            .filter(|capability| *capability != Capability::UnpackedExtensions)
            .collect(),
        BuildSource::FingerprintChromium => {
            let mut capabilities = CHROMIUM_CAPABILITIES.to_vec();
            capabilities.push(Capability::FingerprintConfig);
            capabilities
        }
        BuildSource::Camoufox => vec![
            Capability::Screencast,
            Capability::UnpackedExtensions,
            Capability::Har,
            Capability::RequestInterception,
            Capability::FingerprintConfig,
        ],
        _ => CHROMIUM_CAPABILITIES.to_vec(),
    }
}

pub async fn resolve(
    http: &reqwest::Client,
    urls: &SourceUrls,
    source: BuildSource,
    want: &Want,
) -> Result<Release, Error> {
    resolve_on(http, urls, source, want, std::env::consts::ARCH).await
}

pub async fn available(
    http: &reqwest::Client,
    urls: &SourceUrls,
    source: BuildSource,
) -> Result<Vec<Available>, Error> {
    available_on(http, urls, source, std::env::consts::ARCH).await
}

pub async fn install(
    http: &reqwest::Client,
    release: &Release,
    builds_dir: &Path,
    progress: mpsc::Sender<Progress>,
) -> Result<Installed, Error> {
    let binary = binary_name(release.source)?;
    // The version becomes a directory name that is later deleted.
    let valid = match release.source {
        BuildSource::Camoufox => camoufox::is_version(&release.version),
        _ => is_dotted_number(&release.version),
    };
    if !valid {
        return Err(Error::BuildMissing {
            reason: format!(
                "`{}` is not a browser version; run brr build available",
                release.version
            ),
        });
    }
    let root = builds_dir.join(release.source.as_str());
    let dest = root.join(&release.version);
    let found = {
        let dest = dest.clone();
        task::spawn_blocking(move || extract::find_executable(&dest, binary))
            .await
            .map_err(|err| internal("looking for the installed browser", err))?
    };
    if let Some(relative) = found {
        // The archive is gone, so its hash and size are unknown and the caller keeps its own.
        return Ok(Installed {
            executable: dest.join(relative),
            sha256: None,
            size_bytes: None,
        });
    }

    let name = release
        .url
        .split('?')
        .next()
        .and_then(|url| url.rsplit('/').next())
        .unwrap_or_default()
        .to_owned();
    let kind = ArchiveKind::from_name(&name).ok_or_else(|| Error::Internal {
        message: format!("{name} is not a zip, tar.xz or deb archive"),
    })?;
    if kind != ArchiveKind::Zip {
        task::spawn_blocking(extract::check_xz)
            .await
            .map_err(|err| internal("checking for xz", err))??;
    }
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|err| internal("creating the builds directory", err))?;
    let part = root.join(format!("{}.{}.part", release.version, kind.extension()));
    let downloaded = download(
        http,
        &release.url,
        &part,
        release.sha256.as_deref(),
        Some(progress.clone()),
    )
    .await?;

    progress
        .send(Progress {
            bytes: downloaded.size_bytes,
            total: Some(downloaded.size_bytes),
            phase: InstallPhase::Extracting,
        })
        .await
        .ok();
    let tmp = root.join(format!("{}.tmp", release.version));
    let archive = part.clone();
    let executable =
        task::spawn_blocking(move || install_archive(&archive, kind, &name, &tmp, &dest, binary))
            .await
            .map_err(|err| internal("extracting the archive", err))??;
    tokio::fs::remove_file(&part)
        .await
        .map_err(|err| internal("removing the downloaded archive", err))?;

    Ok(Installed {
        executable,
        sha256: Some(downloaded.sha256),
        size_bytes: Some(downloaded.size_bytes),
    })
}

async fn resolve_on(
    http: &reqwest::Client,
    urls: &SourceUrls,
    source: BuildSource,
    want: &Want,
    arch: &str,
) -> Result<Release, Error> {
    binary_name(source)?;
    if let Want::Channel(channel) = want {
        check_channel(source, channel)?;
    }
    check_arch(source, arch)?;

    match source {
        BuildSource::Cft | BuildSource::HeadlessShell => {
            cft::resolve(http, &urls.cft, source, want, arch).await
        }
        BuildSource::Edge => edge::resolve(http, &urls.edge_repo, want).await,
        BuildSource::Ungoogled | BuildSource::FingerprintChromium | BuildSource::Brave => {
            github::resolve(http, &urls.github_api, source, want, arch).await
        }
        BuildSource::Camoufox => camoufox::resolve(http, &urls.github_api, want, arch).await,
        BuildSource::System => Err(not_installable(source)),
    }
}

async fn available_on(
    http: &reqwest::Client,
    urls: &SourceUrls,
    source: BuildSource,
    arch: &str,
) -> Result<Vec<Available>, Error> {
    binary_name(source)?;
    check_arch(source, arch)?;

    match source {
        BuildSource::Cft | BuildSource::HeadlessShell => {
            cft::available(http, &urls.cft, source, arch).await
        }
        BuildSource::Edge => edge::available(http, &urls.edge_repo).await,
        BuildSource::Ungoogled | BuildSource::FingerprintChromium | BuildSource::Brave => {
            github::available(http, &urls.github_api, source, arch).await
        }
        BuildSource::Camoufox => camoufox::available(http, &urls.github_api, arch).await,
        BuildSource::System => Err(not_installable(source)),
    }
}

// None of the sources brr installs share a binary name.
fn binary_name(source: BuildSource) -> Result<&'static str, Error> {
    match source {
        BuildSource::Cft | BuildSource::Ungoogled | BuildSource::FingerprintChromium => {
            Ok("chrome")
        }
        BuildSource::HeadlessShell => Ok("chrome-headless-shell"),
        BuildSource::Brave => Ok("brave"),
        BuildSource::Edge => Ok("msedge"),
        BuildSource::Camoufox => Ok("camoufox-bin"),
        BuildSource::System => Err(not_installable(source)),
    }
}

fn not_installable(source: BuildSource) -> Error {
    Error::InvalidArgument {
        field: "source".to_owned(),
        message: format!("source {source} cannot be installed by this brr version"),
    }
}

fn check_channel(source: BuildSource, channel: &str) -> Result<(), Error> {
    let offered = channels(source);
    if offered.contains(&channel) {
        return Ok(());
    }

    Err(Error::InvalidArgument {
        field: "channel".to_owned(),
        message: format!("{source} offers: {}", offered.join(", ")),
    })
}

fn check_arch(source: BuildSource, arch: &str) -> Result<(), Error> {
    match (source, arch) {
        (BuildSource::FingerprintChromium | BuildSource::Edge, "x86_64") => Ok(()),
        (BuildSource::FingerprintChromium | BuildSource::Edge, _) => Err(Error::BuildMissing {
            reason: format!(
                "{source} publishes no linux {} build",
                if arch == "aarch64" { "arm64" } else { arch }
            ),
        }),
        _ => Ok(()),
    }
}

// "156.0.8078.79-1" and "v1.99.57" are both published tags.
fn normalize_version(tag: &str) -> &str {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    match tag.rsplit_once('-') {
        Some((version, packaging))
            if !packaging.is_empty() && packaging.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            version
        }
        _ => tag,
    }
}

fn is_dotted_number(version: &str) -> bool {
    version
        .split('.')
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn install_archive(
    archive: &Path,
    kind: ArchiveKind,
    name: &str,
    tmp: &Path,
    dest: &Path,
    binary: &str,
) -> Result<PathBuf, Error> {
    for stale in [tmp, dest] {
        if stale.exists() {
            fs::remove_dir_all(stale).map_err(|err| internal("removing a stale install", err))?;
        }
    }
    fs::create_dir_all(tmp).map_err(|err| internal("creating the install directory", err))?;

    let found = extract(archive, kind, tmp)
        .and_then(|()| {
            extract::find_executable(tmp, binary).ok_or_else(|| Error::Internal {
                message: format!("{binary} not found in {name}"),
            })
        })
        .inspect_err(|_| {
            fs::remove_dir_all(tmp).ok();
        })?;
    fs::rename(tmp, dest).map_err(|err| internal("moving the install into place", err))?;

    Ok(dest.join(found))
}

async fn fetch_capped(request: reqwest::RequestBuilder, url: &str) -> Result<Vec<u8>, Error> {
    let context = format!("fetching {url}");
    let response = request
        .send()
        .await
        .map_err(|err| internal(&context, err))?;
    if let Some(err) = rate_limit_error(&response) {
        return Err(err);
    }
    let response = response
        .error_for_status()
        .map_err(|err| internal(&context, err))?;

    read_capped(response, MAX_JSON_BYTES, &context).await
}

// GitHub signals an exhausted quota with 403 or 429 and x-ratelimit-remaining: 0.
fn rate_limit_error(response: &reqwest::Response) -> Option<Error> {
    if ![StatusCode::FORBIDDEN, StatusCode::TOO_MANY_REQUESTS].contains(&response.status()) {
        return None;
    }
    let header = |name: &str| response.headers().get(name)?.to_str().ok();
    if header(RATE_LIMIT_REMAINING)? != "0" {
        return None;
    }
    let resets = header(RATE_LIMIT_RESET)
        .and_then(|reset| reset.parse::<u64>().ok())
        .map_or_else(
            || "try again later".to_owned(),
            |reset| format!("resets at {}", utc_time(reset)),
        );

    Some(internal("GitHub API rate limit reached", resets))
}

fn utc_time(unix_secs: u64) -> String {
    let time = UtcDateTime::from_unix_ms(i64::try_from(unix_secs).unwrap_or_default() * 1000);

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        time.year, time.month, time.day, time.hour, time.minute, time.second
    )
}

async fn fetch_json<T: DeserializeOwned>(
    request: reqwest::RequestBuilder,
    url: &str,
) -> Result<T, Error> {
    let body = fetch_capped(request, url).await?;

    serde_json::from_slice(&body).map_err(|err| internal(&format!("fetching {url}"), err))
}

async fn read_capped(
    mut response: reqwest::Response,
    limit: usize,
    context: &str,
) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| internal(context, err))?
    {
        if body.len() + chunk.len() > limit {
            return Err(internal(
                context,
                format!("response is larger than {limit} bytes"),
            ));
        }
        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

pub(crate) fn internal(context: &str, err: impl Display) -> Error {
    Error::Internal {
        message: format!("{context}: {err}"),
    }
}
