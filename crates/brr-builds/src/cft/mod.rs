use std::collections::HashMap;

use brr_core::{BuildSource, Error};
use serde::Deserialize;

use crate::{Available, Release, Want, channels, fetch_json};

const LAST_KNOWN_GOOD_FILE: &str = "last-known-good-versions-with-downloads.json";
const KNOWN_GOOD_FILE: &str = "known-good-versions-with-downloads.json";
const AVAILABLE_VERSION_COUNT: usize = 50;
const ARM64_FIRST_MAJOR: u32 = 153;

#[derive(Deserialize)]
struct LastKnownGood {
    channels: HashMap<String, CftVersion>,
}

#[derive(Deserialize)]
struct KnownGood {
    versions: Vec<CftVersion>,
}

#[derive(Deserialize)]
struct CftVersion {
    version: String,
    downloads: Downloads,
}

#[derive(Deserialize)]
struct Downloads {
    #[serde(default)]
    chrome: Vec<Download>,
    #[serde(default, rename = "chrome-headless-shell")]
    headless_shell: Vec<Download>,
}

#[derive(Deserialize)]
struct Download {
    platform: String,
    url: String,
}

struct Platform {
    name: &'static str,
    min_major: u32,
}

fn platform(arch: &str) -> Result<Platform, Error> {
    match arch {
        "x86_64" => Ok(Platform {
            name: "linux64",
            min_major: 0,
        }),
        "aarch64" => Ok(Platform {
            name: "linux-arm64",
            min_major: ARM64_FIRST_MAJOR,
        }),
        _ => Err(Error::BuildMissing {
            reason: format!(
                "Chrome for Testing has no {arch} build; install chromium and run brr build detect"
            ),
        }),
    }
}

fn check_major(version: &str, platform: &Platform) -> Result<(), Error> {
    let major = version
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok());
    if major.is_some_and(|major| major < platform.min_major) {
        return Err(Error::BuildMissing {
            reason: format!(
                "Chrome for Testing has linux arm64 builds from {}; pick a newer version or run brr build detect",
                platform.min_major
            ),
        });
    }

    Ok(())
}

fn download_url<'a>(
    version: &'a CftVersion,
    source: BuildSource,
    platform: &Platform,
) -> Result<&'a str, Error> {
    check_major(&version.version, platform)?;

    let downloads = match source {
        BuildSource::HeadlessShell => &version.downloads.headless_shell,
        _ => &version.downloads.chrome,
    };
    downloads
        .iter()
        .find(|download| download.platform == platform.name)
        .map(|download| download.url.as_str())
        .ok_or_else(|| Error::BuildMissing {
            reason: format!(
                "Chrome for Testing {} has no {} {source} download",
                version.version, platform.name
            ),
        })
}

// The JSON keys channels by capitalized name: "stable" is "Stable".
fn channel_key(channel: &str) -> String {
    let mut chars = channel.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

pub async fn resolve(
    http: &reqwest::Client,
    base_url: &str,
    source: BuildSource,
    want: &Want,
    arch: &str,
) -> Result<Release, Error> {
    let platform = platform(arch)?;
    let base = base_url.trim_end_matches('/');

    let (version, channel) = match want {
        Want::Channel(channel) => {
            let url = format!("{base}/{LAST_KNOWN_GOOD_FILE}");
            let mut doc: LastKnownGood = fetch_json(http.get(&url), &url).await?;
            let version =
                doc.channels
                    .remove(&channel_key(channel))
                    .ok_or_else(|| Error::BuildMissing {
                        reason: format!("Chrome for Testing publishes no {channel} channel"),
                    })?;

            (version, Some(channel.clone()))
        }
        Want::Version(wanted) => {
            check_major(wanted, &platform)?;
            let url = format!("{base}/{KNOWN_GOOD_FILE}");
            let doc: KnownGood = fetch_json(http.get(&url), &url).await?;
            let version = doc
                .versions
                .into_iter()
                .find(|version| version.version == *wanted)
                .ok_or_else(|| Error::BuildMissing {
                    reason: format!(
                        "Chrome for Testing has no version {wanted}; run brr build available"
                    ),
                })?;

            (version, None)
        }
    };

    let url = download_url(&version, source, &platform)?.to_owned();

    Ok(Release {
        source,
        url,
        version: version.version,
        channel,
        sha256: None,
    })
}

pub async fn available(
    http: &reqwest::Client,
    base_url: &str,
    source: BuildSource,
    arch: &str,
) -> Result<Vec<Available>, Error> {
    let platform = platform(arch)?;
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/{LAST_KNOWN_GOOD_FILE}");
    let mut last_known_good: LastKnownGood = fetch_json(http.get(&url), &url).await?;
    let url = format!("{base}/{KNOWN_GOOD_FILE}");
    let known_good: KnownGood = fetch_json(http.get(&url), &url).await?;

    let by_channel = channels(source).iter().filter_map(|name| {
        last_known_good
            .channels
            .remove(&channel_key(name))
            .filter(|version| download_url(version, source, &platform).is_ok())
            .map(|version| Available {
                source,
                channel: Some((*name).to_owned()),
                version: version.version,
            })
    });
    // known-good lists versions oldest first.
    let newest = known_good
        .versions
        .into_iter()
        .rev()
        .filter(|version| download_url(version, source, &platform).is_ok())
        .take(AVAILABLE_VERSION_COUNT)
        .map(|version| Available {
            source,
            channel: None,
            version: version.version,
        });

    Ok(by_channel.chain(newest).collect())
}

#[cfg(test)]
mod tests;
