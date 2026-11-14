use axum::{
    Router,
    http::{HeaderMap, StatusCode, header},
    routing::get,
};
use brr_core::{BuildSource, Error};
use test_case::test_case;

use super::{available, resolve};
use crate::{
    Available, SourceUrls, Want, normalize_version, resolve as resolve_any, test_support::serve,
};

const UNGOOGLED: &str = include_str!("../../tests/fixtures/github-ungoogled-releases.json");
const FINGERPRINT: &str =
    include_str!("../../tests/fixtures/github-fingerprint-chromium-releases.json");
const BRAVE: &str = include_str!("../../tests/fixtures/github-brave-releases.json");
const X86_64: &str = "x86_64";
const AARCH64: &str = "aarch64";

// GitHub answers 403 without a User-Agent.
async fn releases(headers: HeaderMap, body: &'static str) -> (StatusCode, &'static str) {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    let accept = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok());
    if user_agent.is_some_and(|agent| agent.starts_with("brr/"))
        && accept == Some("application/vnd.github+json")
    {
        (StatusCode::OK, body)
    } else {
        (StatusCode::FORBIDDEN, "")
    }
}

async fn serve_fixtures() -> String {
    serve(
        Router::new()
            .route(
                "/repos/ungoogled-software/ungoogled-chromium-portablelinux/releases",
                get(|headers| releases(headers, UNGOOGLED)),
            )
            .route(
                "/repos/adryfish/fingerprint-chromium/releases",
                get(|headers| releases(headers, FINGERPRINT)),
            )
            .route(
                "/repos/brave/brave-browser/releases",
                get(|headers| releases(headers, BRAVE)),
            ),
    )
    .await
}

fn stable() -> Want {
    Want::Channel("stable".into())
}

#[test_case(
    BuildSource::Ungoogled,
    "156.0.8078.79",
    "-x86_64_linux.tar.xz",
    "d35a9064068274af1a2efa8bcb9b42306e590f9b5c929789f2e0e51a35880499"
)]
#[test_case(
    BuildSource::FingerprintChromium,
    "150.0.7871.186",
    "-x86_64_linux.tar.xz",
    "48eff860053efd9ad498d51370f0a1f59f983df39c02a72ad2a8682cdead4055"
)]
#[test_case(
    BuildSource::Brave,
    "1.99.57",
    "-linux-amd64.zip",
    "734bd9e1d24bc0a94c03b0a43845107dd8a29f572c652023cf7177f5fc3a5f38"
)]
#[tokio::test]
async fn stable_resolves_the_x86_64_asset(
    source: BuildSource,
    version: &str,
    suffix: &str,
    sha256: &str,
) {
    let api = serve_fixtures().await;

    let release = resolve(&reqwest::Client::new(), &api, source, &stable(), X86_64)
        .await
        .unwrap();

    assert_eq!(release.source, source);
    assert_eq!(release.version, version);
    assert_eq!(release.channel.as_deref(), Some("stable"));
    assert!(release.url.ends_with(suffix), "{}", release.url);
    assert_eq!(release.sha256.as_deref(), Some(sha256));
}

#[tokio::test]
async fn a_version_picks_that_release_and_skips_prereleases() {
    let api = serve_fixtures().await;
    let http = reqwest::Client::new();

    let older = resolve(
        &http,
        &api,
        BuildSource::Ungoogled,
        &Want::Version("156.0.8078.73".into()),
        X86_64,
    )
    .await
    .unwrap();
    let prerelease = resolve(
        &http,
        &api,
        BuildSource::Brave,
        &Want::Version("1.99.58".into()),
        X86_64,
    )
    .await;

    assert!(older.url.contains("156.0.8078.73-1-x86_64_linux.tar.xz"));
    assert_eq!(older.channel, None);
    assert!(matches!(prerelease, Err(Error::BuildMissing { .. })));
}

#[tokio::test]
async fn arm64_picks_the_arm_asset() {
    let api = serve_fixtures().await;
    let http = reqwest::Client::new();

    let ungoogled = resolve(&http, &api, BuildSource::Ungoogled, &stable(), AARCH64)
        .await
        .unwrap();
    let brave = resolve(&http, &api, BuildSource::Brave, &stable(), AARCH64)
        .await
        .unwrap();

    assert!(ungoogled.url.ends_with("-arm64_linux.tar.xz"));
    assert!(brave.url.ends_with("brave-browser-1.99.57-linux-arm64.zip"));
}

#[tokio::test]
async fn available_lists_versions_newest_first_with_stable_on_the_first() {
    let api = serve_fixtures().await;

    let versions = available(&reqwest::Client::new(), &api, BuildSource::Brave, X86_64)
        .await
        .unwrap();

    assert_eq!(
        versions,
        [
            Available {
                source: BuildSource::Brave,
                channel: Some("stable".into()),
                version: "1.99.57".into()
            },
            Available {
                source: BuildSource::Brave,
                channel: None,
                version: "1.99.56".into()
            },
        ]
    );
}

#[test_case("156.0.8078.79-1", "156.0.8078.79")]
#[test_case("v1.99.57", "1.99.57")]
#[test_case("150.0.7871.186", "150.0.7871.186")]
#[test_case("v1.2.3-4", "1.2.3")]
fn tags_are_normalized(tag: &str, version: &str) {
    assert_eq!(normalize_version(tag), version);
}

#[tokio::test]
async fn brave_stable_skips_nightly_and_beta_builds_labelled_as_releases() {
    let api = serve_fixtures().await;
    let http = reqwest::Client::new();

    let release = resolve(&http, &api, BuildSource::Brave, &stable(), X86_64)
        .await
        .unwrap();
    let nightly = resolve(
        &http,
        &api,
        BuildSource::Brave,
        &Want::Version("1.102.2".into()),
        X86_64,
    )
    .await;
    let versions = available(&http, &api, BuildSource::Brave, X86_64)
        .await
        .unwrap();

    assert_eq!(release.version, "1.99.57");
    assert!(matches!(nightly, Err(Error::BuildMissing { .. })));
    assert_eq!(versions.len(), 2);
}

#[tokio::test]
async fn brave_beta_channel_is_invalid() {
    let urls = SourceUrls {
        cft: String::new(),
        github_api: "http://127.0.0.1:1".into(),
        edge_repo: String::new(),
    };

    let result = resolve_any(
        &reqwest::Client::new(),
        &urls,
        BuildSource::Brave,
        &Want::Channel("beta".into()),
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        Error::InvalidArgument {
            field: "channel".into(),
            message: "brave offers: stable".into()
        }
    );
}

#[tokio::test]
async fn releases_without_an_asset_for_the_arch_are_not_listed() {
    let api = serve_fixtures().await;
    let http = reqwest::Client::new();

    let versions: Vec<_> = available(&http, &api, BuildSource::FingerprintChromium, X86_64)
        .await
        .unwrap()
        .into_iter()
        .map(|available| available.version)
        .collect();

    assert_eq!(
        versions,
        ["150.0.7871.186", "148.0.7778.215", "144.0.7559.132"]
    );
}

#[test_case(StatusCode::FORBIDDEN)]
#[test_case(StatusCode::TOO_MANY_REQUESTS)]
#[tokio::test]
async fn an_exhausted_rate_limit_says_when_it_resets(status: StatusCode) {
    let api = serve(Router::new().route(
        "/repos/brave/brave-browser/releases",
        get(move || async move {
            (
                status,
                [
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", "1788000000"),
                ],
                "",
            )
        }),
    ))
    .await;

    let result = resolve(
        &reqwest::Client::new(),
        &api,
        BuildSource::Brave,
        &stable(),
        X86_64,
    )
    .await;

    assert_eq!(
        result.unwrap_err(),
        Error::Internal {
            message: "GitHub API rate limit reached: resets at 2026-08-29 10:40:00 UTC".into()
        }
    );
}
