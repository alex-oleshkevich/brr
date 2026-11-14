use axum::{Router, routing::get};
use brr_core::{BuildSource, Capability, Error};
use test_case::test_case;
use tokio::sync::mpsc;

use crate::{
    Release, SourceUrls, Want, available_on, capabilities, channels, fetch_json, install,
    read_capped, resolve_on, test_support::serve,
};

const UNREACHABLE: &str = "http://127.0.0.1:1";
const AARCH64: &str = "aarch64";
const BIG_BODY: &str = r#"{"padding":"0123456789"}"#;

fn unreachable_urls() -> SourceUrls {
    SourceUrls {
        cft: UNREACHABLE.into(),
        github_api: UNREACHABLE.into(),
        edge_repo: UNREACHABLE.into(),
    }
}

#[test_case(BuildSource::Cft, &["stable", "beta", "dev", "canary"])]
#[test_case(BuildSource::HeadlessShell, &["stable", "beta", "dev", "canary"])]
#[test_case(BuildSource::Brave, &["stable"])]
#[test_case(BuildSource::Edge, &["stable"])]
#[test_case(BuildSource::System, &[])]
#[test_case(BuildSource::Camoufox, &["latest-compatible"])]
fn channels_follow_the_table(source: BuildSource, expected: &[&str]) {
    assert_eq!(channels(source), expected);
}

#[test_case(BuildSource::FingerprintChromium)]
#[test_case(BuildSource::Edge)]
#[tokio::test]
async fn arm64_rejects_x86_only_sources_before_any_request(source: BuildSource) {
    let http = reqwest::Client::new();
    let urls = unreachable_urls();
    let want = Want::Channel("stable".into());

    let resolved = resolve_on(&http, &urls, source, &want, AARCH64).await;
    let listed = available_on(&http, &urls, source, AARCH64).await;

    for result in [resolved.map(|_| ()), listed.map(|_| ())] {
        assert_eq!(
            result.unwrap_err(),
            Error::BuildMissing {
                reason: format!("{source} publishes no linux arm64 build")
            }
        );
    }
}

#[tokio::test]
async fn install_rejects_a_version_that_is_not_dotted_digits() {
    let builds = tempfile::tempdir().unwrap();
    let (sender, _receiver) = mpsc::channel(1);

    for version in ["", "..", "1.2/../3", "1..2", "abc"] {
        let release = Release {
            source: BuildSource::Cft,
            version: version.into(),
            channel: None,
            url: format!("{UNREACHABLE}/x.zip"),
            sha256: None,
        };

        let result = install(
            &reqwest::Client::new(),
            &release,
            builds.path(),
            sender.clone(),
        )
        .await;

        let Err(Error::BuildMissing { reason }) = result else {
            panic!("expected BuildMissing for {version:?}");
        };
        assert!(reason.contains("not a browser version"), "{version}");
    }
}

#[tokio::test]
async fn json_body_is_capped() {
    let base = serve(Router::new().route("/big", get(|| async { BIG_BODY }))).await;
    let url = format!("{base}/big");
    let http = reqwest::Client::new();

    let capped = read_capped(
        http.get(&url).send().await.unwrap(),
        BIG_BODY.len() - 1,
        "t",
    )
    .await;
    let exact = read_capped(http.get(&url).send().await.unwrap(), BIG_BODY.len(), "t").await;

    assert!(capped.is_err());
    assert_eq!(exact.unwrap().len(), BIG_BODY.len());
    assert!(
        fetch_json::<serde_json::Value>(http.get(&url), &url)
            .await
            .is_ok()
    );
}

#[test_case(BuildSource::Cft, true, false)]
#[test_case(BuildSource::System, true, false)]
#[test_case(BuildSource::HeadlessShell, false, false)]
#[test_case(BuildSource::FingerprintChromium, true, true)]
#[test_case(BuildSource::Camoufox, true, true)]
fn capabilities_follow_the_table(
    source: BuildSource,
    unpacked_extensions: bool,
    fingerprint_config: bool,
) {
    let caps = capabilities(source);

    assert_eq!(
        caps.contains(&Capability::UnpackedExtensions),
        unpacked_extensions
    );
    assert_eq!(
        caps.contains(&Capability::FingerprintConfig),
        fingerprint_config
    );
}

#[test]
fn camoufox_has_no_pdf() {
    assert!(!capabilities(BuildSource::Camoufox).contains(&Capability::Pdf));
}
