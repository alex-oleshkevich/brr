use std::{
    fs,
    io::{Cursor, Write},
    process::Command,
};

use axum::Router;
use tokio::net::TcpListener;
use zip::{ZipWriter, write::SimpleFileOptions};

pub async fn serve(router: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    format!("http://{addr}")
}

pub fn zip_bytes(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content, mode) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default().unix_permissions(*mode))
            .unwrap();
        writer.write_all(content).unwrap();
    }

    writer.finish().unwrap().into_inner()
}

#[allow(clippy::disallowed_macros)]
pub fn has_xz() -> bool {
    let found = Command::new("xz").arg("--version").output().is_ok();
    if !found {
        eprintln!("skipped: xz is not on PATH");
    }

    found
}

// A fonts bundle: everything sits under fonts/, as in the published release.
pub fn fonts_tar_xz() -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("src/fonts");
    fs::create_dir_all(root.join("L")).unwrap();
    fs::write(root.join("groups.json"), "{}").unwrap();
    fs::write(root.join("L/a.ttf"), "font").unwrap();
    let out = dir.path().join("bundle.tar.xz");
    let status = Command::new("tar")
        .arg("-cJf")
        .arg(&out)
        .arg("-C")
        .arg(dir.path().join("src"))
        .arg("fonts")
        .status()
        .unwrap();
    assert!(status.success());

    fs::read(out).unwrap()
}
