use std::{fs, os::unix::fs::symlink};

use super::*;

const FILE_BYTES: usize = 1000;

#[tokio::test]
async fn dir_size_sums_files_and_skips_symlinks() {
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("big"), vec![0; 10 * FILE_BYTES]).unwrap();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    fs::write(dir.path().join("a"), vec![0; FILE_BYTES]).unwrap();
    fs::write(dir.path().join("nested/b"), vec![0; FILE_BYTES]).unwrap();
    symlink(outside.path(), dir.path().join("link")).unwrap();

    assert_eq!(
        dir_size(dir.path().to_path_buf()).await,
        2 * FILE_BYTES as u64
    );
}

#[test]
fn free_bytes_reads_the_filesystem() {
    let dir = tempfile::tempdir().unwrap();

    assert!(free_bytes(dir.path()).unwrap() > 0);
}
