use axum::{Router, routing::get};
use brr_core::{BuildSource, Error};
use test_case::test_case;

use super::{available, resolve};
use crate::{
    Available, SourceUrls, Want, available as available_any, resolve as resolve_any,
    test_support::serve,
};

const LAST_KNOWN_GOOD: &str =
    include_str!("../../tests/fixtures/last-known-good-versions-with-downloads.json");
const KNOWN_GOOD: &str =
    include_str!("../../tests/fixtures/known-good-versions-with-downloads.json");
const UNREACHABLE: &str = "http://127.0.0.1:1";
const X86_64: &str = "x86_64";
const AARCH64: &str = "aarch64";
const STABLE_VERSION: &str = "157.0.8103.78";
const STABLE_URL: &str = "https://storage.googleapis.com/chrome-for-testing-public/157.0.8103.78/linux64/chrome-linux64.zip";
const HEADLESS_SHELL_URL: &str = "https://storage.googleapis.com/chrome-for-testing-public/157.0.8103.78/linux64/chrome-headless-shell-linux64.zip";
const OLDER_VERSION: &str = "160.0.8156.0";
const NEWER_VERSION: &str = "160.0.8157.0";

async fn serve_fixtures() -> String {
    serve(
        Router::new()
            .route(
                "/last-known-good-versions-with-downloads.json",
                get(|| async { LAST_KNOWN_GOOD }),
            )
            .route(
                "/known-good-versions-with-downloads.json",
                get(|| async { KNOWN_GOOD }),
            ),
    )
    .await
}

fn is_build_missing(result: Result<impl std::fmt::Debug, Error>) -> String {
    match result {
        Err(Error::BuildMissing { reason }) => reason,
        other => panic!("expected BuildMissing, got {other:?}"),
    }
}

#[tokio::test]
async fn resolves_a_channel() {
    let base = serve_fixtures().await;

    let release = resolve(
        &reqwest::Client::new(),
        &base,
        BuildSource::Cft,
        &Want::Channel("stable".into()),
        X86_64,
    )
    .await
    .unwrap();

    assert_eq!(release.version, STABLE_VERSION);
    assert_eq!(release.url, STABLE_URL);
    assert_eq!(release.channel.as_deref(), Some("stable"));
}

#[tokio::test]
async fn resolves_a_version() {
    let base = serve_fixtures().await;

    let release = resolve(
        &reqwest::Client::new(),
        &base,
        BuildSource::Cft,
        &Want::Version(OLDER_VERSION.into()),
        X86_64,
    )
    .await
    .unwrap();

    assert_eq!(release.version, OLDER_VERSION);
    assert!(release.url.contains(&format!("{OLDER_VERSION}/linux64/")));
    assert_eq!(release.channel, None);
}

#[tokio::test]
async fn unknown_channel_and_version_are_build_missing() {
    let base = serve_fixtures().await;
    let http = reqwest::Client::new();

    let channel = is_build_missing(
        resolve(
            &http,
            &base,
            BuildSource::Cft,
            &Want::Channel("nightly".into()),
            X86_64,
        )
        .await,
    );
    let version = is_build_missing(
        resolve(
            &http,
            &base,
            BuildSource::Cft,
            &Want::Version("1.2.3.4".into()),
            X86_64,
        )
        .await,
    );

    assert!(channel.contains("nightly"));
    assert!(version.contains("1.2.3.4"));
}

#[tokio::test]
async fn version_without_linux64_is_build_missing() {
    let json = r#"{"versions":[{"version":"9.9.9.9","downloads":{"chrome":[{"platform":"mac-arm64","url":"u"}]}},{"version":"9.9.9.8","downloads":{}}]}"#;
    let base = serve(Router::new().route(
        "/known-good-versions-with-downloads.json",
        get(move || async move { json }),
    ))
    .await;
    let http = reqwest::Client::new();

    let reason = is_build_missing(
        resolve(
            &http,
            &base,
            BuildSource::Cft,
            &Want::Version("9.9.9.9".into()),
            X86_64,
        )
        .await,
    );

    assert!(reason.contains("9.9.9.9") && reason.contains("linux64"));
}

#[tokio::test]
async fn available_lists_channels_then_newest_first() {
    let base = serve_fixtures().await;

    let available = available(&reqwest::Client::new(), &base, BuildSource::Cft, X86_64)
        .await
        .unwrap();

    let channels: Vec<_> = available
        .iter()
        .filter_map(|a| a.channel.as_deref())
        .collect();
    assert_eq!(channels, ["stable", "beta", "dev", "canary"]);
    assert_eq!(
        available[4..],
        [
            Available {
                source: BuildSource::Cft,
                channel: None,
                version: NEWER_VERSION.into()
            },
            Available {
                source: BuildSource::Cft,
                channel: None,
                version: OLDER_VERSION.into()
            },
        ]
    );
}

#[tokio::test]
async fn arm64_picks_the_arm_download() {
    let base = serve_fixtures().await;

    let release = resolve(
        &reqwest::Client::new(),
        &base,
        BuildSource::Cft,
        &Want::Channel("stable".into()),
        AARCH64,
    )
    .await
    .unwrap();

    assert_eq!(release.version, STABLE_VERSION);
    assert!(release.url.contains("linux-arm64/chrome-linux-arm64.zip"));
}

#[tokio::test]
async fn arm64_rejects_versions_before_153_and_unknown_arches() {
    let json = r#"{"versions":[{"version":"152.0.1.1","downloads":{"chrome":[{"platform":"linux-arm64","url":"u"}]}}]}"#;
    let base = serve(Router::new().route(
        "/known-good-versions-with-downloads.json",
        get(move || async move { json }),
    ))
    .await;
    let http = reqwest::Client::new();
    let want = Want::Version("152.0.1.1".into());

    let old = is_build_missing(resolve(&http, &base, BuildSource::Cft, &want, AARCH64).await);
    let arch = is_build_missing(resolve(&http, &base, BuildSource::Cft, &want, "riscv64").await);

    assert!(old.contains("arm64") && old.contains("153"));
    assert!(arch.contains("riscv64") && arch.contains("brr build detect"));
}

#[tokio::test]
async fn available_applies_the_arch_filter() {
    let json = r#"{"versions":[{"version":"160.0.1.1","downloads":{"chrome":[{"platform":"linux-arm64","url":"u"}]}},{"version":"152.0.1.1","downloads":{"chrome":[{"platform":"linux-arm64","url":"u"}]}},{"version":"150.0.1.1","downloads":{"chrome":[{"platform":"linux64","url":"u"}]}}]}"#;
    let base = serve(
        Router::new()
            .route(
                "/last-known-good-versions-with-downloads.json",
                get(|| async { LAST_KNOWN_GOOD }),
            )
            .route(
                "/known-good-versions-with-downloads.json",
                get(move || async move { json }),
            ),
    )
    .await;

    let versions: Vec<_> = available(&reqwest::Client::new(), &base, BuildSource::Cft, AARCH64)
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.channel.is_none())
        .map(|a| a.version)
        .collect();

    assert_eq!(versions, ["160.0.1.1"]);
}

#[tokio::test]
async fn headless_shell_resolves_its_own_download() {
    let base = serve_fixtures().await;

    let release = resolve(
        &reqwest::Client::new(),
        &base,
        BuildSource::HeadlessShell,
        &Want::Channel("stable".into()),
        X86_64,
    )
    .await
    .unwrap();

    assert_eq!(release.source, BuildSource::HeadlessShell);
    assert_eq!(release.version, STABLE_VERSION);
    assert_eq!(release.url, HEADLESS_SHELL_URL);
}

#[tokio::test]
async fn arm64_version_below_153_is_rejected_before_any_request() {
    let result = resolve(
        &reqwest::Client::new(),
        "http://127.0.0.1:1",
        BuildSource::Cft,
        &Want::Version("152.0.1.1".into()),
        AARCH64,
    )
    .await;

    assert!(is_build_missing(result).contains("153"));
}

#[test_case(BuildSource::System)]
#[tokio::test]
async fn sources_without_an_installer_are_invalid(source: BuildSource) {
    let http = reqwest::Client::new();
    let urls = SourceUrls {
        cft: UNREACHABLE.into(),
        github_api: UNREACHABLE.into(),
        edge_repo: UNREACHABLE.into(),
    };

    let resolved = resolve_any(&http, &urls, source, &Want::Channel("stable".into())).await;
    let listed = available_any(&http, &urls, source).await;

    for result in [resolved.map(|_| ()), listed.map(|_| ())] {
        assert_eq!(
            result.unwrap_err(),
            Error::InvalidArgument {
                field: "source".into(),
                message: format!("source {source} cannot be installed by this brr version")
            }
        );
    }
}
