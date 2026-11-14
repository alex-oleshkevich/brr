use std::{cmp::Ordering, fs, path::Path};

use brr_core::{BuildSource, Error};
use serde::{Deserialize, Serialize};

use crate::{
    Available, Release, Want,
    github::{self, Asset},
    internal,
};

pub const CAMOUFOX_REPO: &str = "daijro/camoufox";
// The Juggler line of Playwright 1.62.
pub const CAMOUFOX_MIN_VERSION: &str = "158.0.1-beta.41";
pub(crate) const RELEASES_PER_PAGE: usize = 50;
const LATEST_COMPATIBLE: &str = "latest-compatible";
const COMPAT_FILE: &str = "brr-compat.json";

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompatRecord {
    pub passed: bool,
    pub checked_at: i64,
    pub failed_step: Option<String>,
    pub brr_version: String,
}

pub fn read_compat(build_dir: &Path) -> Option<CompatRecord> {
    serde_json::from_slice(&fs::read(build_dir.join(COMPAT_FILE)).ok()?).ok()
}

pub fn write_compat(build_dir: &Path, record: &CompatRecord) -> Result<(), Error> {
    let context = "writing the compatibility record";
    let body = serde_json::to_vec(record).map_err(|err| internal(context, err))?;
    let tmp = build_dir.join(format!("{COMPAT_FILE}.tmp"));
    fs::write(&tmp, body).map_err(|err| internal(context, err))?;

    fs::rename(&tmp, build_dir.join(COMPAT_FILE)).map_err(|err| internal(context, err))
}

// <major>.<minor>[.<patch>]-beta.<n>, as the numbers of the dotted part and the beta number.
fn parse_version(version: &str) -> Option<(Vec<u64>, u64)> {
    let (dotted, beta) = version.split_once("-beta.")?;
    let number = |part: &str| {
        part.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| part.parse().ok())
            .flatten()
    };
    let parts: Vec<u64> = dotted.split('.').map(number).collect::<Option<_>>()?;

    ((2..=3).contains(&parts.len())).then_some((parts, number(beta)?))
}

pub(crate) fn is_version(version: &str) -> bool {
    parse_version(version).is_some()
}

pub fn camoufox_version_cmp(a: &str, b: &str) -> Ordering {
    parse_version(a).cmp(&parse_version(b))
}

fn is_supported(version: &str) -> bool {
    camoufox_version_cmp(version, CAMOUFOX_MIN_VERSION) != Ordering::Less
}

fn arch_name(arch: &str) -> Result<&'static str, Error> {
    match arch {
        "x86_64" => Ok("x86_64"),
        "aarch64" => Ok("arm64"),
        _ => Err(Error::BuildMissing {
            reason: format!("camoufox publishes no linux {arch} build"),
        }),
    }
}

// Supported releases, newest first, each with the one zip of this architecture.
async fn builds(
    http: &reqwest::Client,
    api_url: &str,
    arch: &str,
) -> Result<Vec<(String, Asset)>, Error> {
    let arch = arch_name(arch)?;
    let releases = github::releases(http, api_url, CAMOUFOX_REPO, RELEASES_PER_PAGE).await?;
    let mut builds: Vec<_> = releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| {
            let version = release.tag_name.strip_prefix('v')?.to_owned();
            if !is_supported(&version) {
                return None;
            }
            let name = format!("camoufox-{version}-lin.{arch}.zip");
            let asset = release
                .assets
                .into_iter()
                .find(|asset| asset.name == name)?;

            Some((version, asset))
        })
        .collect();
    builds.sort_by(|(a, _), (b, _)| camoufox_version_cmp(b, a));

    Ok(builds)
}

pub async fn resolve(
    http: &reqwest::Client,
    api_url: &str,
    want: &Want,
    arch: &str,
) -> Result<Release, Error> {
    let (wanted, channel) = match want {
        Want::Channel(channel) => (None, Some(channel.clone())),
        Want::Version(version) => {
            if !is_version(version) {
                return Err(Error::BuildMissing {
                    reason: format!(
                        "`{version}` is not a Camoufox version; run brr build available"
                    ),
                });
            }
            if !is_supported(version) {
                return Err(Error::BuildMissing {
                    reason: format!(
                        "Camoufox {version} is older than the supported range (>= {CAMOUFOX_MIN_VERSION})"
                    ),
                });
            }
            (Some(version), None)
        }
    };

    let mut builds = builds(http, api_url, arch).await?.into_iter();
    let (version, asset) = match wanted {
        None => builds.next(),
        Some(wanted) => builds.find(|(version, _)| version == wanted),
    }
    .ok_or_else(|| Error::BuildMissing {
        reason: match wanted {
            None => format!("camoufox has no supported release for linux {arch}"),
            Some(wanted) => format!(
                "camoufox has no version {wanted} for linux {arch}; run brr build available"
            ),
        },
    })?;

    Ok(Release {
        source: BuildSource::Camoufox,
        version,
        channel,
        sha256: asset.sha256(),
        url: asset.browser_download_url,
    })
}

pub async fn available(
    http: &reqwest::Client,
    api_url: &str,
    arch: &str,
) -> Result<Vec<Available>, Error> {
    let builds = builds(http, api_url, arch).await?;

    Ok(builds
        .into_iter()
        .enumerate()
        .map(|(index, (version, _))| Available {
            source: BuildSource::Camoufox,
            channel: (index == 0).then(|| LATEST_COMPATIBLE.to_owned()),
            version,
        })
        .collect())
}

#[cfg(test)]
mod tests;
