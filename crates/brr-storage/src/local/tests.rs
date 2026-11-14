use brr_core::Error;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

use super::Local;
use crate::{Backend, validate_key};

const KEY: &str = "artifacts/a/f.png";

fn is_invalid(result: Result<(), Error>) -> bool {
    matches!(result, Err(Error::InvalidArgument { .. }))
}

#[tokio::test]
async fn local_roundtrip_and_key_guard() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("data");
    let backend = Backend::Local(Local::new(root.clone()));
    let data = b"png bytes".to_vec();
    let expected = Sha256::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    let stored = backend.put_bytes(KEY, data.clone().into()).await.unwrap();
    assert_eq!(stored.size_bytes, data.len() as u64);
    assert_eq!(stored.sha256, expected);

    let path = backend.local_path(KEY).unwrap().unwrap();
    assert_eq!(path, root.join(KEY));
    assert_eq!(std::fs::read(&path).unwrap(), data);

    let copy = dir.path().join("out/copy.png");
    assert_eq!(
        backend.get_to_file(KEY, &copy).await.unwrap(),
        data.len() as u64
    );
    assert_eq!(std::fs::read(&copy).unwrap(), data);
    assert_eq!(backend.get_bytes(KEY, 9).await.unwrap(), data);
    assert!(matches!(
        backend.get_bytes(KEY, 8).await,
        Err(Error::StorageUnavailable { .. })
    ));

    let source = dir.path().join("source.bin");
    std::fs::write(&source, &data).unwrap();
    assert_eq!(
        backend.put_file("artifacts/b", &source).await.unwrap(),
        stored
    );

    backend.delete(KEY).await.unwrap();
    backend.delete(KEY).await.unwrap();
    assert!(!path.exists());
    assert_eq!(
        backend
            .presign_get(KEY, std::time::Duration::from_secs(60))
            .await
            .unwrap(),
        None
    );

    let before = walk(dir.path());
    for bad in ["artifacts/../../x", "/abs", "a\0b", ""] {
        assert!(is_invalid(validate_key(bad)), "{bad:?}");
        assert!(backend.local_path(bad).is_err(), "{bad:?}");
        assert!(
            backend.put_bytes(bad, data.clone().into()).await.is_err(),
            "{bad:?}"
        );
        assert!(is_invalid(backend.delete(bad).await), "{bad:?}");
    }
    assert_eq!(walk(dir.path()), before);
    assert!(!dir.path().join("x").exists());
}

#[tokio::test]
async fn probe_succeeds_and_leaves_nothing() {
    let dir = tempdir().unwrap();
    let backend = Backend::Local(Local::new(dir.path().to_path_buf()));

    backend.test().await.unwrap();

    let left = walk(dir.path());
    assert!(left.is_empty(), "{left:?}");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        found.push(path.clone());
        if path.is_dir() {
            found.extend(walk(&path));
        }
    }
    found.sort();
    found
}

#[tokio::test]
async fn put_owned_file_moves_the_source_and_reports_its_hash() {
    let dir = tempdir().unwrap();
    let backend = Backend::Local(Local::new(dir.path().join("data")));
    let source = dir.path().join("download.bin");
    std::fs::write(&source, b"payload").unwrap();

    let stored = backend.put_owned_file(KEY, &source).await.unwrap();

    assert!(!source.exists());
    assert_eq!(stored.size_bytes, 7);
    assert_eq!(stored.sha256, crate::sha256_hex(b"payload"));
    assert_eq!(backend.get_bytes(KEY, 7).await.unwrap(), b"payload");
}

#[test]
fn clear_temp_removes_only_unrenamed_temp_files() {
    let dir = tempdir().unwrap();
    let nested = dir.path().join("artifacts/a");
    std::fs::create_dir_all(&nested).unwrap();
    let stale = nested.join(".brr-tmp-f.png.01ARZ3NDEKTSV4RRFFQ69G5FAV.tmp");
    let lookalike = nested.join(".report.tmp");
    let kept = nested.join("f.png");
    std::fs::write(&stale, b"half").unwrap();
    std::fs::write(&kept, b"whole").unwrap();
    std::fs::write(&lookalike, b"user file").unwrap();

    assert_eq!(crate::clear_temp(dir.path()), 1);
    assert!(!stale.exists());
    assert!(kept.exists() && lookalike.exists());
}
