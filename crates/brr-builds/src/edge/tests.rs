use axum::{Router, routing::get};
use brr_core::BuildSource;

use super::{available, resolve};
use crate::{Available, Want, test_support::serve};

const PACKAGES: &str = include_str!("../../tests/fixtures/edge-packages.txt");
const PACKAGES_ROUTE: &str = "/dists/stable/main/binary-amd64/Packages";
const STABLE_SHA256: &str = "3e53834d9d6e505bc57e6bff44d92cfe8cf0bed9064e88efc5b0d8f1c75f70fa";

async fn serve_fixture() -> String {
    serve(Router::new().route(PACKAGES_ROUTE, get(|| async { PACKAGES }))).await
}

#[tokio::test]
async fn stable_is_the_highest_version() {
    let repo = serve_fixture().await;

    let release = resolve(
        &reqwest::Client::new(),
        &repo,
        &Want::Channel("stable".into()),
    )
    .await
    .unwrap();

    assert_eq!(release.source, BuildSource::Edge);
    assert_eq!(release.version, "142.0.1.2");
    assert_eq!(release.channel.as_deref(), Some("stable"));
    assert_eq!(
        release.url,
        format!(
            "{repo}/pool/main/m/microsoft-edge-stable/microsoft-edge-stable_142.0.1.2-1_amd64.deb"
        )
    );
    assert_eq!(release.sha256.as_deref(), Some(STABLE_SHA256));
}

#[tokio::test]
async fn a_version_picks_its_stanza() {
    let repo = serve_fixture().await;

    let release = resolve(
        &reqwest::Client::new(),
        &repo,
        &Want::Version("141.0.9.9".into()),
    )
    .await
    .unwrap();

    assert_eq!(release.version, "141.0.9.9");
    assert_ne!(release.sha256.as_deref(), Some(STABLE_SHA256));
}

#[tokio::test]
async fn available_lists_newest_first() {
    let repo = serve_fixture().await;

    let versions = available(&reqwest::Client::new(), &repo).await.unwrap();

    assert_eq!(
        versions,
        [
            Available {
                source: BuildSource::Edge,
                channel: Some("stable".into()),
                version: "142.0.1.2".into()
            },
            Available {
                source: BuildSource::Edge,
                channel: None,
                version: "141.0.9.9".into()
            },
        ]
    );
}
