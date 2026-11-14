use std::{
    fs,
    io::Cursor,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::Command,
};

use brr_core::Error;
use test_case::test_case;

use super::{ArchiveKind, ar_member, extract, find_executable};
use crate::test_support::zip_bytes;

const CHROME_MODE: u32 = 0o755;

#[allow(clippy::disallowed_macros)]
fn has_xz() -> bool {
    let found = Command::new("xz").arg("--version").output().is_ok();
    if !found {
        eprintln!("skipped: xz is not on PATH");
    }

    found
}

fn ar_bytes(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = b"!<arch>\n".to_vec();
    for (name, data) in members {
        let header = format!(
            "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            0,
            0,
            0,
            100644,
            data.len()
        );
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(data);
        if data.len() % 2 == 1 {
            bytes.push(b'\n');
        }
    }

    bytes
}

fn tar_xz(dir: &Path, source: &Path) -> std::path::PathBuf {
    let archive = dir.join("a.tar.xz");
    let status = Command::new("tar")
        .arg("-cJf")
        .arg(&archive)
        .arg("-C")
        .arg(source)
        .arg(".")
        .status()
        .unwrap();
    assert!(status.success());

    archive
}

fn chrome_tree(root: &Path) {
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("chrome"), b"#!/bin/sh\n").unwrap();
    fs::set_permissions(bin.join("chrome"), fs::Permissions::from_mode(CHROME_MODE)).unwrap();
}

#[test_case("a.zip", Some(ArchiveKind::Zip))]
#[test_case("addon.xpi", Some(ArchiveKind::Zip))]
#[test_case("a.tar.xz", Some(ArchiveKind::TarXz))]
#[test_case("edge.deb", Some(ArchiveKind::Deb))]
#[test_case("a.tar.gz", None)]
fn archive_kind_follows_the_suffix(name: &str, kind: Option<ArchiveKind>) {
    assert_eq!(ArchiveKind::from_name(name), kind);
}

#[test]
fn deb_reader_finds_the_data_member_despite_padding() {
    let data = b"data-bytes!";
    let deb = ar_bytes(&[
        ("debian-binary", b"2.0\n"),
        ("control.tar.xz", b"ctl"),
        ("data.tar.xz/ ", data),
    ]);
    let mut reader = Cursor::new(deb);

    let length = ar_member(&mut reader, "data.tar.xz").unwrap().unwrap();

    let start = reader.position() as usize;
    assert_eq!(&reader.get_ref()[start..start + length as usize], data);
}

#[test]
fn deb_reader_reports_a_missing_member_and_bad_magic() {
    let deb = ar_bytes(&[("debian-binary", b"2.0\n")]);

    assert_eq!(
        ar_member(&mut Cursor::new(deb), "data.tar.xz").unwrap(),
        None
    );
    assert!(ar_member(&mut Cursor::new(b"not an archive".to_vec()), "data.tar.xz").is_err());
}

#[test]
fn tar_xz_extracts_an_executable() {
    if !has_xz() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src");
    chrome_tree(&source);
    let archive = tar_xz(dir.path(), &source);
    let dest = dir.path().join("out");
    fs::create_dir_all(&dest).unwrap();

    extract(&archive, ArchiveKind::TarXz, &dest).unwrap();

    let found = find_executable(&dest, "chrome").unwrap();
    assert_eq!(found, Path::new("bin/chrome"));
    let mode = fs::metadata(dest.join(found)).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, CHROME_MODE);
}

#[test]
fn deb_extracts_its_data_member() {
    if !has_xz() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src/opt/microsoft/msedge");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("msedge"), b"x").unwrap();
    let data = fs::read(tar_xz(dir.path(), &dir.path().join("src"))).unwrap();
    let deb = dir.path().join("edge.deb");
    fs::write(
        &deb,
        ar_bytes(&[("debian-binary", b"2.0\n"), ("data.tar.xz/", &data)]),
    )
    .unwrap();
    let dest = dir.path().join("out");
    fs::create_dir_all(&dest).unwrap();

    extract(&deb, ArchiveKind::Deb, &dest).unwrap();

    assert_eq!(
        find_executable(&dest, "msedge").unwrap(),
        Path::new("opt/microsoft/msedge/msedge")
    );
    assert!(!dir.path().join("out.data.tar.xz").exists());
}

#[test]
fn a_corrupt_tar_xz_is_an_internal_error() {
    if !has_xz() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("a.tar.xz");
    fs::write(&archive, b"garbage").unwrap();

    let result = extract(&archive, ArchiveKind::TarXz, dir.path());

    assert!(matches!(result, Err(Error::Internal { .. })));
}

#[test]
fn walk_is_bounded_sorted_and_ignores_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    let outside = dir.path().join("outside");
    fs::create_dir_all(root.join("a/b/c/d")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(root.join("a/b/c/d/chrome"), b"x").unwrap();
    fs::write(outside.join("chrome"), b"x").unwrap();
    symlink(&outside, root.join("a/link")).unwrap();

    assert_eq!(find_executable(&root, "chrome"), None);

    fs::create_dir_all(root.join("a/b/c")).unwrap();
    fs::write(root.join("a/b/c/chrome"), b"x").unwrap();
    assert_eq!(
        find_executable(&root, "chrome").unwrap(),
        Path::new("a/b/c/chrome")
    );
}

#[test]
fn zip_traversal_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("a.zip");
    fs::write(
        &archive,
        zip_bytes(&[
            ("../evil", b"x", 0o644),
            ("/abs-evil", b"x", 0o644),
            ("chrome-linux64/chrome", b"x", CHROME_MODE),
        ]),
    )
    .unwrap();
    let dest = dir.path().join("out/dest");
    fs::create_dir_all(&dest).unwrap();

    extract(&archive, ArchiveKind::Zip, &dest).unwrap();

    assert!(!dest.parent().unwrap().join("evil").exists());
    assert!(!dir.path().join("evil").exists());
    assert!(!Path::new("/abs-evil").exists());
    let chrome = dest.join("chrome-linux64/chrome");
    assert_eq!(
        fs::metadata(chrome).unwrap().permissions().mode() & 0o777,
        CHROME_MODE
    );
}
