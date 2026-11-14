use std::{cmp::Ordering, fs, os::unix::fs::PermissionsExt};

use axum::{Router, http::StatusCode, routing::get};
use brr_core::{BuildSource, Error, sha256_hex};
use test_case::test_case;
use tokio::sync::mpsc;

use super::{CompatRecord, camoufox_version_cmp, read_compat, write_compat};
use crate::{
    Release, SourceUrls, Want, available_on, fonts_bundle_release, install, install_fonts_bundle,
    install_geoip, resolve_on,
    test_support::{fonts_tar_xz, has_xz, serve, zip_bytes},
};

const RELEASES: &str = include_str!("fixtures/releases.json");
const X86_64: &str = "x86_64";
const AARCH64: &str = "aarch64";

fn urls(base: &str) -> SourceUrls {
    SourceUrls {
        cft: String::new(),
        github_api: base.to_owned(),
        edge_repo: String::new(),
    }
}

async fn serve_releases() -> String {
    serve(Router::new().route(
        "/repos/daijro/camoufox/releases",
        get(|| async { RELEASES }),
    ))
    .await
}

#[test_case(X86_64, "-lin.x86_64.zip", "a")]
#[test_case(AARCH64, "-lin.arm64.zip", "b")]
#[tokio::test]
async fn releases(arch: &str, suffix: &str, digest_char: &str) {
    let urls = urls(&serve_releases().await);
    let http = reqwest::Client::new();

    let listed = available_on(&http, &urls, BuildSource::Camoufox, arch)
        .await
        .unwrap();
    let latest = resolve_on(
        &http,
        &urls,
        BuildSource::Camoufox,
        &Want::Channel("latest-compatible".into()),
        arch,
    )
    .await
    .unwrap();
    let pinned = resolve_on(
        &http,
        &urls,
        BuildSource::Camoufox,
        &Want::Version("158.0.1-beta.41".into()),
        arch,
    )
    .await
    .unwrap();

    let versions: Vec<_> = listed.iter().map(|a| a.version.as_str()).collect();
    assert_eq!(versions, ["158.0.1-beta.43", "158.0.1-beta.41"]);
    assert_eq!(listed[0].channel.as_deref(), Some("latest-compatible"));
    assert_eq!(listed[1].channel, None);
    assert_eq!(latest.version, "158.0.1-beta.43");
    assert_eq!(latest.channel.as_deref(), Some("latest-compatible"));
    assert!(latest.url.ends_with(suffix), "{}", latest.url);
    assert_eq!(latest.sha256, Some(digest_char.repeat(64)));
    assert_eq!(pinned.version, "158.0.1-beta.41");
    assert_eq!(pinned.channel, None);
}

#[tokio::test]
async fn releases_outside_the_range_and_other_channels_are_refused() {
    let urls = urls(&serve_releases().await);
    let http = reqwest::Client::new();

    let old = resolve_on(
        &http,
        &urls,
        BuildSource::Camoufox,
        &Want::Version("156.0.1-beta.36".into()),
        X86_64,
    )
    .await;
    let missing = resolve_on(
        &http,
        &urls,
        BuildSource::Camoufox,
        &Want::Version("999.0.1-beta.1".into()),
        X86_64,
    )
    .await;
    let stable = resolve_on(
        &http,
        &urls,
        BuildSource::Camoufox,
        &Want::Channel("stable".into()),
        X86_64,
    )
    .await;

    assert_eq!(
        old.unwrap_err(),
        Error::BuildMissing {
            reason:
                "Camoufox 156.0.1-beta.36 is older than the supported range (>= 158.0.1-beta.41)"
                    .into()
        }
    );
    assert!(matches!(missing, Err(Error::BuildMissing { .. })));
    assert!(matches!(
        stable,
        Err(Error::InvalidArgument { field, .. }) if field == "channel"
    ));
}

#[test]
fn version_order() {
    let ordered = ["158.0.1-beta.41", "158.0-beta.40", "156.0.1-beta.39"];

    assert_eq!(
        camoufox_version_cmp(ordered[0], ordered[1]),
        Ordering::Greater
    );
    assert_eq!(
        camoufox_version_cmp(ordered[1], ordered[2]),
        Ordering::Greater
    );
    assert_eq!(
        camoufox_version_cmp("158.0.1-beta.9", "158.0.1-beta.41"),
        Ordering::Less
    );
    assert_eq!(
        camoufox_version_cmp("158.0.1-beta.41", "158.0.1-beta.41"),
        Ordering::Equal
    );
}

#[tokio::test]
async fn install_and_assets() {
    let zip = zip_bytes(&[
        ("camoufox-bin", b"#!/bin/sh\n", 0o755),
        ("properties.json", b"{}", 0o644),
    ]);
    let geoip = zip_bytes(&[("nested/db.mmdb", b"mmdb", 0o644)]);
    let two_dbs = zip_bytes(&[("a.mmdb", b"1", 0o644), ("b.mmdb", b"2", 0o644)]);
    let (zip_body, geoip_body, two_body) = (zip.clone(), geoip, two_dbs);
    let base = serve(
        Router::new()
            .route("/camoufox.zip", get(move || async move { zip_body }))
            .route("/geoip.zip", get(move || async move { geoip_body }))
            .route("/two.zip", get(move || async move { two_body }))
            .route("/gone", get(|| async { StatusCode::NOT_FOUND })),
    )
    .await;
    let http = reqwest::Client::new();
    let builds = tempfile::tempdir().unwrap();
    let (sender, _receiver) = mpsc::channel(64);
    let release = Release {
        source: BuildSource::Camoufox,
        version: "158.0.1-beta.41".into(),
        channel: None,
        url: format!("{base}/camoufox.zip"),
        sha256: Some("0".repeat(64)),
    };

    let bad = install(&http, &release, builds.path(), sender.clone()).await;
    let build_dir = builds.path().join("camoufox/158.0.1-beta.41");
    assert!(bad.is_err());
    assert!(!build_dir.exists());

    let release = Release {
        sha256: Some(sha256_hex(&zip)),
        ..release
    };
    let installed = install(&http, &release, builds.path(), sender.clone())
        .await
        .unwrap();
    assert_eq!(installed.executable, build_dir.join("camoufox-bin"));
    assert_eq!(installed.sha256, release.sha256);
    assert_eq!(installed.size_bytes, Some(zip.len() as u64));
    let mode = fs::metadata(&installed.executable)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111);
    assert!(
        !builds
            .path()
            .join("camoufox/158.0.1-beta.41.zip.part")
            .exists()
    );

    let record = CompatRecord {
        passed: false,
        checked_at: 1_700_000_000,
        failed_step: Some("screencast".into()),
        brr_version: "0.1.0".into(),
    };
    assert_eq!(read_compat(&build_dir), None);
    write_compat(&build_dir, &record).unwrap();
    assert_eq!(read_compat(&build_dir), Some(record));

    let assets = tempfile::tempdir().unwrap();
    let db = install_geoip(&http, &format!("{base}/geoip.zip"), assets.path())
        .await
        .unwrap();
    assert_eq!(db, assets.path().join("geoip-aio-all.mmdb"));
    assert_eq!(fs::read(&db).unwrap(), b"mmdb");
    assert!(
        install_geoip(&http, &format!("{base}/two.zip"), assets.path())
            .await
            .is_err()
    );
    assert!(
        install_geoip(&http, &format!("{base}/gone"), assets.path())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn install_fonts_bundle_unpacks_the_fonts_directory() {
    if !has_xz() {
        return;
    }
    let tar = fonts_tar_xz();
    let tar_body = tar.clone();
    let base =
        serve(Router::new().route("/fonts.tar.xz", get(move || async move { tar_body }))).await;
    let http = reqwest::Client::new();
    let assets = tempfile::tempdir().unwrap();
    let (sender, _receiver) = mpsc::channel(64);
    let bundle = crate::AssetRelease {
        version: "v4".into(),
        url: format!("{base}/fonts.tar.xz"),
        sha256: Some("0".repeat(64)),
        size: None,
    };
    let dir = assets.path().join("fonts-bundle-v4");

    let bad = install_fonts_bundle(&http, &bundle, &dir, sender.clone()).await;
    assert!(bad.is_err());
    assert!(!dir.exists());

    let bundle = crate::AssetRelease {
        sha256: Some(sha256_hex(&tar)),
        ..bundle
    };
    let bytes = install_fonts_bundle(&http, &bundle, &dir, sender)
        .await
        .unwrap();
    assert_eq!(bytes, "{}".len() as u64 + "font".len() as u64);
    assert!(dir.join("groups.json").is_file());
    assert!(dir.join("L/a.ttf").is_file());
}

#[tokio::test]
async fn fonts_bundle_release_is_the_newest_font_bundle() {
    let urls = urls(&serve_releases().await);

    let release = fonts_bundle_release(&reqwest::Client::new(), &urls)
        .await
        .unwrap();

    assert_eq!(release.version, "v4");
    assert!(release.url.ends_with("/fonts-bundle-v4.tar.xz"));
    assert_eq!(release.sha256, Some("f".repeat(64)));
    assert_eq!(release.size, Some(900_000_000));
}
