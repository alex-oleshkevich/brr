use std::{path::Path, time::Duration};

use brr_core::{Error, Secret, model::BackendKind, settings::StorageSettings};
use reqwest::Url;

use super::S3;
use crate::{Backend, Config, S3Config, config, r2_endpoint};

fn r2_settings() -> StorageSettings {
    StorageSettings {
        backend: BackendKind::R2,
        bucket: Some("b".into()),
        access_key_id: Some("k".into()),
        r2_account_id: Some("abc123".into()),
        ..StorageSettings::default()
    }
}

fn s3_config(result: Result<Config, Error>) -> S3Config {
    match result.unwrap() {
        Config::S3(config) => config,
        Config::Local { .. } => panic!("expected an S3 config"),
    }
}

fn field(result: Result<Config, Error>) -> String {
    match result.unwrap_err() {
        Error::InvalidArgument { field, .. } => field,
        other => panic!("unexpected error {other:?}"),
    }
}

#[test]
fn config_derivation() {
    let root = Path::new("/data");
    let secret = || Some(Secret::new("s"));

    let r2 = s3_config(config(&r2_settings(), secret(), root));
    assert_eq!(r2.endpoint, "https://abc123.r2.cloudflarestorage.com");
    assert_eq!(r2.endpoint, r2_endpoint("abc123"));
    assert_eq!(r2.region, "auto");
    assert!(r2.path_style);
    assert_eq!(r2.kind, BackendKind::R2);

    let mut explicit = r2_settings();
    explicit.endpoint = Some(String::new());
    assert!(config(&explicit, secret(), root).is_ok());
    explicit.endpoint = Some("https://x.example".into());
    assert_eq!(field(config(&explicit, secret(), root)), "endpoint");

    let mut s3 = StorageSettings {
        backend: BackendKind::S3,
        endpoint: Some("http://localhost:9000".into()),
        bucket: Some("b".into()),
        access_key_id: Some("k".into()),
        public_endpoint: Some("https://cdn.example.com".into()),
        ..StorageSettings::default()
    };
    s3.public_endpoint = Some(String::new());
    assert_eq!(s3_config(config(&s3, secret(), root)).public_endpoint, None);
    s3.public_endpoint = Some("https://cdn.example.com".into());
    let derived = s3_config(config(&s3, secret(), root));
    assert_eq!(derived.region, "us-east-1");
    assert!(!derived.path_style);
    assert_eq!(
        derived.public_endpoint.as_deref(),
        Some("https://cdn.example.com")
    );

    assert_eq!(field(config(&s3, None, root)), "secret_access_key");
    s3.bucket = None;
    assert_eq!(field(config(&s3, secret(), root)), "bucket");

    let local = config(&StorageSettings::default(), None, root).unwrap();
    assert!(matches!(local, Config::Local { root: found } if found == root));
}

#[test]
fn debug_hides_the_secret() {
    let settings = r2_settings();
    let config = s3_config(config(
        &settings,
        Some(Secret::new("hunter2hunter2")),
        Path::new("/"),
    ));

    assert!(!format!("{config:?}").contains("hunter2"));
}

#[tokio::test]
async fn presign_uses_public_endpoint() {
    let s3 = S3::new(S3Config {
        kind: BackendKind::S3,
        endpoint: "http://127.0.0.1:9".into(),
        region: "us-east-1".into(),
        bucket: "bucket".into(),
        access_key_id: "k".into(),
        secret_access_key: Secret::new("s"),
        path_style: true,
        public_endpoint: Some("https://cdn.example.com".into()),
    })
    .unwrap();

    let url = s3
        .presign_get("artifacts/x", Duration::from_secs(3600))
        .await
        .unwrap();
    let url = Url::parse(&url).unwrap();

    assert_eq!(url.host_str(), Some("cdn.example.com"));
    assert_eq!(url.path(), "/bucket/artifacts/x");
    assert!(
        url.query_pairs()
            .any(|(k, v)| k == "X-Amz-Expires" && v == "3600")
    );
}

#[tokio::test]
async fn virtual_hosted_presign_puts_bucket_in_host() {
    let s3 = S3::new(S3Config {
        kind: BackendKind::S3,
        endpoint: "http://s3.example.test:9000".into(),
        region: "us-east-1".into(),
        bucket: "b".into(),
        access_key_id: "k".into(),
        secret_access_key: Secret::new("s"),
        path_style: false,
        public_endpoint: None,
    })
    .unwrap();

    let url = s3
        .presign_get("artifacts/x", Duration::from_secs(60))
        .await
        .unwrap();
    let url = Url::parse(&url).unwrap();

    assert_eq!(url.host_str(), Some("b.s3.example.test"));
    assert_eq!(url.path(), "/artifacts/x");
}

#[test]
fn holds_follows_endpoint_and_bucket() {
    let secret = || Some(Secret::new("s"));
    let settings = |endpoint: &str, bucket: &str| StorageSettings {
        backend: BackendKind::S3,
        endpoint: Some(endpoint.into()),
        bucket: Some(bucket.into()),
        access_key_id: Some("k".into()),
        ..StorageSettings::default()
    };
    let open = |settings: &StorageSettings| {
        Backend::open(config(settings, secret(), Path::new("/data")).unwrap()).unwrap()
    };

    let base = open(&settings("http://a.example:9000", "one"));
    let same = open(&settings("http://a.example:9000/", "one"));
    let other_bucket = open(&settings("http://a.example:9000", "two"));
    let other_endpoint = open(&settings("http://b.example:9000", "one"));

    let location = format!("{}artifacts/a/f.png", base.key_prefix());

    assert_eq!(base.key_prefix(), same.key_prefix());
    assert!(base.holds("s3", &location));
    assert!(same.holds("s3", &location));
    assert!(!other_bucket.holds("s3", &location));
    assert!(!other_endpoint.holds("s3", &location));
    assert!(!base.holds("r2", &location));
}
