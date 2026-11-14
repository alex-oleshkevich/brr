#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use std::{io::Write, time::Duration};

use brr_core::{Error, Secret, model::BackendKind};
use brr_storage::{Backend, Config, S3Config};
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use ulid::Ulid;

const BIG_FILE_BYTES: usize = 12 << 20;

struct Env {
    endpoint: String,
    key: String,
    secret: String,
    bucket: String,
}

fn env() -> Option<Env> {
    let get = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let found = (|| {
        Some(Env {
            endpoint: get("BRR_TEST_S3_ENDPOINT")?,
            key: get("BRR_TEST_S3_KEY")?,
            secret: get("BRR_TEST_S3_SECRET")?,
            bucket: get("BRR_TEST_S3_BUCKET")?,
        })
    })();
    if found.is_none() {
        eprintln!(
            "skipped: set BRR_TEST_S3_ENDPOINT, BRR_TEST_S3_KEY, BRR_TEST_S3_SECRET and BRR_TEST_S3_BUCKET to run the live S3 tests"
        );
    }

    found
}

fn backend(env: &Env, secret: &str) -> Backend {
    Backend::open(Config::S3(S3Config {
        kind: BackendKind::S3,
        endpoint: env.endpoint.clone(),
        region: "us-east-1".into(),
        bucket: env.bucket.clone(),
        access_key_id: env.key.clone(),
        secret_access_key: Secret::new(secret),
        path_style: true,
        public_endpoint: None,
    }))
    .unwrap()
}

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test]
async fn upload_presign_download_delete() {
    let Some(env) = env() else { return };
    let backend = backend(&env, &env.secret);
    let key = format!("brr-test/{}/big.bin", Ulid::generate());

    // xorshift: incompressible enough, and no rand dependency for a test file
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let data: Vec<u8> = (0..BIG_FILE_BYTES)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect();
    let dir = tempdir().unwrap();
    let source = dir.path().join("big.bin");
    std::fs::File::create(&source)
        .unwrap()
        .write_all(&data)
        .unwrap();

    let stored = backend.put_file(&key, &source).await.unwrap();
    assert_eq!(stored.size_bytes, BIG_FILE_BYTES as u64);
    assert_eq!(stored.sha256, sha256(&data));

    let url = backend
        .presign_get(&key, Duration::from_secs(60))
        .await
        .unwrap()
        .unwrap();
    let response = reqwest::get(&url).await.unwrap();
    assert!(response.status().is_success());
    assert_eq!(sha256(&response.bytes().await.unwrap()), stored.sha256);

    let copy = dir.path().join("copy.bin");
    assert_eq!(
        backend.get_to_file(&key, &copy).await.unwrap(),
        stored.size_bytes
    );
    assert_eq!(sha256(&std::fs::read(&copy).unwrap()), stored.sha256);
    assert!(matches!(
        backend.get_bytes(&key, 1024).await,
        Err(Error::StorageUnavailable { .. })
    ));

    backend.delete(&key).await.unwrap();
    backend.delete(&key).await.unwrap();
    assert_eq!(reqwest::get(&url).await.unwrap().status(), 404);
}

#[tokio::test]
async fn connection_test() {
    let Some(env) = env() else { return };

    backend(&env, &env.secret).test().await.unwrap();

    match backend(&env, "wrong-secret").test().await {
        Err(Error::StorageUnavailable { message }) => assert!(message.contains("put"), "{message}"),
        other => panic!("expected StorageUnavailable, got {other:?}"),
    }
}
