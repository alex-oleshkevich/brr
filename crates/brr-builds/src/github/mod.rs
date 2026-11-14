use brr_core::{BuildSource, Error};
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;

use crate::{Available, Release, Want, fetch_json, normalize_version};

const RELEASES_PER_PAGE: usize = 100;
const AVAILABLE_VERSION_COUNT: usize = 30;
const SHA256_PREFIX: &str = "sha256:";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";

#[derive(Deserialize)]
pub(crate) struct GithubRelease {
    pub(crate) tag_name: String,
    #[serde(default)]
    pub(crate) draft: bool,
    #[serde(default)]
    pub(crate) prerelease: bool,
    #[serde(default)]
    pub(crate) assets: Vec<Asset>,
}

#[derive(Deserialize)]
pub(crate) struct Asset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
    digest: Option<String>,
    pub(crate) size: Option<u64>,
}

impl Asset {
    pub(crate) fn sha256(&self) -> Option<String> {
        self.digest
            .as_deref()?
            .strip_prefix(SHA256_PREFIX)
            .map(str::to_owned)
    }
}

// One request per call: the unauthenticated API allows 60 an hour.
pub(crate) async fn releases(
    http: &reqwest::Client,
    api_url: &str,
    repo: &str,
    per_page: usize,
) -> Result<Vec<GithubRelease>, Error> {
    let url = format!(
        "{}/repos/{repo}/releases?per_page={per_page}",
        api_url.trim_end_matches('/')
    );
    let request = http
        .get(&url)
        .header(USER_AGENT, crate::USER_AGENT)
        .header(ACCEPT, GITHUB_ACCEPT);

    fetch_json(request, &url).await
}

// None for sources GitHub does not host and for architectures a repository does not publish.
fn spec(source: BuildSource, arch: &str) -> Option<(&'static str, &'static str)> {
    const UNGOOGLED: &str = "ungoogled-software/ungoogled-chromium-portablelinux";
    const FINGERPRINT: &str = "adryfish/fingerprint-chromium";
    const BRAVE: &str = "brave/brave-browser";

    match (source, arch) {
        (BuildSource::Ungoogled, "x86_64") => Some((UNGOOGLED, "-x86_64_linux.tar.xz")),
        (BuildSource::Ungoogled, "aarch64") => Some((UNGOOGLED, "-arm64_linux.tar.xz")),
        (BuildSource::FingerprintChromium, "x86_64") => Some((FINGERPRINT, "-x86_64_linux.tar.xz")),
        (BuildSource::Brave, "x86_64") => Some((BRAVE, "-linux-amd64.zip")),
        (BuildSource::Brave, "aarch64") => Some((BRAVE, "-linux-arm64.zip")),
        (BuildSource::Ungoogled | BuildSource::FingerprintChromium | BuildSource::Brave, _)
        | (
            BuildSource::Cft
            | BuildSource::HeadlessShell
            | BuildSource::Edge
            | BuildSource::System
            | BuildSource::Camoufox,
            _,
        ) => None,
    }
}

// Brave labels Nightly and Beta builds as plain releases, but only stable ships exactly
// brave-browser-<version>-linux-<arch>.zip; the others carry a channel name in the file name.
fn is_build_asset(source: BuildSource, name: &str, version: &str, suffix: &str) -> bool {
    match source {
        BuildSource::Brave => name == format!("brave-browser-{version}{suffix}"),
        _ => name.ends_with(suffix),
    }
}

// Newest first, with the one download of this architecture each release offers.
async fn builds(
    http: &reqwest::Client,
    api_url: &str,
    source: BuildSource,
    arch: &str,
) -> Result<Vec<(String, Asset)>, Error> {
    let (repo, suffix) = spec(source, arch).ok_or_else(|| Error::BuildMissing {
        reason: format!("{source} publishes no linux {arch} build"),
    })?;
    let releases = releases(http, api_url, repo, RELEASES_PER_PAGE).await?;

    Ok(releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| {
            let version = normalize_version(&release.tag_name).to_owned();
            let asset = release
                .assets
                .into_iter()
                .find(|asset| is_build_asset(source, &asset.name, &version, suffix))?;

            Some((version, asset))
        })
        .collect())
}

pub async fn resolve(
    http: &reqwest::Client,
    api_url: &str,
    source: BuildSource,
    want: &Want,
    arch: &str,
) -> Result<Release, Error> {
    let mut builds = builds(http, api_url, source, arch).await?.into_iter();
    let (build, channel) = match want {
        Want::Channel(channel) => (builds.next(), Some(channel.clone())),
        Want::Version(wanted) => (builds.find(|(version, _)| version == wanted), None),
    };
    let (version, asset) = build.ok_or_else(|| Error::BuildMissing {
        reason: match want {
            Want::Channel(_) => format!("{source} has no stable release for linux {arch}"),
            Want::Version(wanted) => format!(
                "{source} has no version {wanted} for linux {arch}; run brr build available"
            ),
        },
    })?;

    Ok(Release {
        source,
        version,
        channel,
        sha256: asset.sha256(),
        url: asset.browser_download_url,
    })
}

pub async fn available(
    http: &reqwest::Client,
    api_url: &str,
    source: BuildSource,
    arch: &str,
) -> Result<Vec<Available>, Error> {
    let builds = builds(http, api_url, source, arch).await?;

    Ok(builds
        .into_iter()
        .take(AVAILABLE_VERSION_COUNT)
        .enumerate()
        .map(|(index, (version, _))| Available {
            source,
            channel: (index == 0).then(|| "stable".to_owned()),
            version,
        })
        .collect())
}

#[cfg(test)]
mod tests;
