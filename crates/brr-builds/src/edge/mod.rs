use brr_core::{BuildSource, Error, compare_versions};

use crate::{Available, Release, Want, fetch_capped, internal, normalize_version};

const PACKAGES_PATH: &str = "dists/stable/main/binary-amd64/Packages";
const PACKAGE_NAME: &str = "microsoft-edge-stable";
const AVAILABLE_VERSION_COUNT: usize = 30;

struct Package {
    version: String,
    filename: String,
    sha256: String,
}

fn parse_packages(index: &str) -> Vec<Package> {
    index
        .split("\n\n")
        .filter_map(|stanza| {
            let (mut name, mut version, mut filename, mut sha256) = (None, None, None, None);
            // Continuation lines start with a space and carry no key.
            for line in stanza.lines().filter(|line| !line.starts_with(' ')) {
                match line.split_once(": ") {
                    Some(("Package", value)) => name = Some(value),
                    Some(("Version", value)) => version = Some(normalize_version(value)),
                    Some(("Filename", value)) => filename = Some(value),
                    Some(("SHA256", value)) => sha256 = Some(value),
                    _ => {}
                }
            }

            if name? != PACKAGE_NAME {
                return None;
            }

            Some(Package {
                version: version?.to_owned(),
                filename: filename?.to_owned(),
                sha256: sha256?.to_owned(),
            })
        })
        .collect()
}

// Newest first.
async fn packages(http: &reqwest::Client, repo_url: &str) -> Result<Vec<Package>, Error> {
    let repo = repo_url.trim_end_matches('/');
    let url = format!("{repo}/{PACKAGES_PATH}");
    let body = fetch_capped(http.get(&url), &url).await?;
    let index = String::from_utf8(body).map_err(|err| internal(&format!("reading {url}"), err))?;

    let mut packages = parse_packages(&index);
    packages.sort_by(|a, b| compare_versions(&b.version, &a.version));

    Ok(packages)
}

pub async fn resolve(
    http: &reqwest::Client,
    repo_url: &str,
    want: &Want,
) -> Result<Release, Error> {
    let packages = packages(http, repo_url).await?;
    let (package, channel) = match want {
        Want::Channel(channel) => (packages.into_iter().next(), Some(channel.clone())),
        Want::Version(wanted) => (
            packages
                .into_iter()
                .find(|package| package.version == *wanted),
            None,
        ),
    };
    let package = package.ok_or_else(|| Error::BuildMissing {
        reason: match want {
            Want::Channel(_) => "the Edge repository lists no stable release".to_owned(),
            Want::Version(wanted) => {
                format!("the Edge repository has no version {wanted}; run brr build available")
            }
        },
    })?;

    Ok(Release {
        source: BuildSource::Edge,
        version: package.version,
        channel,
        url: format!("{}/{}", repo_url.trim_end_matches('/'), package.filename),
        sha256: Some(package.sha256),
    })
}

pub async fn available(http: &reqwest::Client, repo_url: &str) -> Result<Vec<Available>, Error> {
    let packages = packages(http, repo_url).await?;

    Ok(packages
        .into_iter()
        .take(AVAILABLE_VERSION_COUNT)
        .enumerate()
        .map(|(index, package)| Available {
            source: BuildSource::Edge,
            channel: (index == 0).then(|| "stable".to_owned()),
            version: package.version,
        })
        .collect())
}

#[cfg(test)]
mod tests;
