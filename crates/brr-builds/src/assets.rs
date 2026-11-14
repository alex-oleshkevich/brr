use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

use brr_core::Error;
use tokio::{sync::mpsc, task};

use crate::{
    ArchiveKind, CAMOUFOX_REPO, Progress, SourceUrls, camoufox::RELEASES_PER_PAGE, download,
    extract, github, internal,
};

pub const GEOIP_URL: &str =
    "https://github.com/daijro/geoip-all-in-one/releases/latest/download/geoip-aio-all.mmdb.zip";
const FONTS_TAG_PREFIX: &str = "font-bundle-v";
const GEOIP_FILE: &str = "geoip-aio-all.mmdb";
const GROUPS_FILE: &str = "groups.json";
const BUNDLE_ROOT: &str = "fonts";

#[derive(Debug, Clone, PartialEq)]
pub struct AssetRelease {
    pub version: String,
    pub url: String,
    pub sha256: Option<String>,
    pub size: Option<u64>,
}

pub async fn fonts_bundle_release(
    http: &reqwest::Client,
    urls: &SourceUrls,
) -> Result<AssetRelease, Error> {
    let releases =
        github::releases(http, &urls.github_api, CAMOUFOX_REPO, RELEASES_PER_PAGE).await?;
    let newest = releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| {
            let number = release.tag_name.strip_prefix(FONTS_TAG_PREFIX)?;
            if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            let number: u64 = number.parse().ok()?;
            let name = format!("fonts-bundle-v{number}.tar.xz");
            let asset = release
                .assets
                .into_iter()
                .find(|asset| asset.name == name)?;

            Some((number, asset))
        })
        .max_by_key(|(number, _)| *number);
    let (number, asset) = newest.ok_or_else(|| Error::BuildMissing {
        reason: "camoufox has no fonts bundle release".to_owned(),
    })?;

    Ok(AssetRelease {
        version: format!("v{number}"),
        sha256: asset.sha256(),
        size: asset.size,
        url: asset.browser_download_url,
    })
}

pub async fn install_fonts_bundle(
    http: &reqwest::Client,
    release: &AssetRelease,
    dir: &Path,
    progress: mpsc::Sender<Progress>,
) -> Result<u64, Error> {
    if dir.join(GROUPS_FILE).is_file() {
        let measured = dir.to_owned();

        return task::spawn_blocking(move || disk_bytes(&measured))
            .await
            .map_err(|err| internal("measuring the fonts bundle", err))?
            .map_err(|err| internal("measuring the fonts bundle", err));
    }
    task::spawn_blocking(crate::extract::check_xz)
        .await
        .map_err(|err| internal("checking for xz", err))??;
    let part = sibling(dir, ".tar.xz.part");
    let tmp = sibling(dir, ".tmp");
    if let Some(parent) = dir.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|err| internal("creating the assets directory", err))?;
    }
    download(
        http,
        &release.url,
        &part,
        release.sha256.as_deref(),
        Some(progress),
    )
    .await?;

    let dest = dir.to_owned();
    let bytes = task::spawn_blocking(move || {
        let result = unpack(&part, ArchiveKind::TarXz, &tmp).and_then(|()| {
            let root = tmp.join(BUNDLE_ROOT);
            if !root.join(GROUPS_FILE).is_file() {
                return Err(Error::BuildMissing {
                    reason: "fonts bundle has no groups.json".to_owned(),
                });
            }
            replace(&root, &dest)?;
            disk_bytes(&dest).map_err(|err| internal("measuring the fonts bundle", err))
        });
        fs::remove_dir_all(&tmp).ok();
        fs::remove_file(&part).ok();

        result
    })
    .await
    .map_err(|err| internal("extracting the fonts bundle", err))??;

    Ok(bytes)
}

pub async fn install_geoip(
    http: &reqwest::Client,
    url: &str,
    dir: &Path,
) -> Result<PathBuf, Error> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|err| internal("creating the assets directory", err))?;
    // The source publishes no checksum.
    let part = dir.join(format!("{GEOIP_FILE}.zip.part"));
    download(http, url, &part, None, None).await?;

    let tmp = dir.join(format!("{GEOIP_FILE}.tmp"));
    let target = dir.join(GEOIP_FILE);
    let result = task::spawn_blocking(move || {
        let result = unpack(&part, ArchiveKind::Zip, &tmp).and_then(|()| {
            let mut found = files(&tmp)
                .map_err(|err| internal("listing the GeoIP archive", err))?
                .into_iter()
                .map(|(path, _)| path)
                .filter(|path| path.extension().is_some_and(|ext| ext == "mmdb"));
            let (Some(mmdb), None) = (found.next(), found.next()) else {
                return Err(Error::Internal {
                    message: "the GeoIP archive must hold exactly one .mmdb file".to_owned(),
                });
            };
            fs::rename(mmdb, &target)
                .map_err(|err| internal("installing the GeoIP database", err))?;

            Ok(target)
        });
        fs::remove_dir_all(&tmp).ok();
        fs::remove_file(&part).ok();

        result
    })
    .await
    .map_err(|err| internal("extracting the GeoIP archive", err))??;

    Ok(result)
}

// `dir` plus a suffix, in the same parent so the final rename stays on one filesystem.
fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(dir);
    name.push(suffix);

    PathBuf::from(name)
}

fn unpack(archive: &Path, kind: ArchiveKind, tmp: &Path) -> Result<(), Error> {
    if tmp.exists() {
        fs::remove_dir_all(tmp).map_err(|err| internal("removing a stale install", err))?;
    }
    fs::create_dir_all(tmp).map_err(|err| internal("creating the install directory", err))?;

    extract(archive, kind, tmp)
}

fn replace(root: &Path, dest: &Path) -> Result<(), Error> {
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|err| internal("removing a stale install", err))?;
    }

    fs::rename(root, dest).map_err(|err| internal("moving the install into place", err))
}

// Every regular file under `dir`; symlinks are neither followed nor counted.
fn files(dir: &Path) -> io::Result<Vec<(PathBuf, u64)>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            found.extend(files(&entry.path())?);
        } else if file_type.is_file() {
            found.push((entry.path(), entry.metadata()?.len()));
        }
    }

    Ok(found)
}

fn disk_bytes(dir: &Path) -> io::Result<u64> {
    Ok(files(dir)?.iter().map(|(_, size)| size).sum())
}
