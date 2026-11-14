use std::{
    fs,
    io::Read,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use super::{MAX_UPLOAD_BYTES, MIB, TAIL_CHUNK, read_input, tail_lines, write_output};

fn decode(value: &Value) -> Vec<u8> {
    STANDARD
        .decode(value.as_str().expect("string"))
        .expect("base64")
}

#[test]
fn dir_as_tar() {
    let dir = tempfile::tempdir().expect("dir");
    fs::write(dir.path().join("manifest.json"), "{}").expect("write");
    symlink("/etc/passwd", dir.path().join("link")).expect("symlink");

    let tar_bytes = decode(&read_input(dir.path(), false).expect("tar"));
    let mut archive = tar::Archive::new(tar_bytes.as_slice());
    let mut entries = Vec::new();
    for entry in archive.entries().expect("entries") {
        let mut entry = entry.expect("entry");
        let mut content = Vec::new();
        entry.read_to_end(&mut content).expect("read");
        entries.push((
            entry.path().expect("path").display().to_string(),
            entry.header().entry_type().is_symlink(),
            content,
        ));
    }

    assert!(entries.iter().any(|(path, _, _)| path == "manifest.json"));
    let link = entries
        .iter()
        .find(|(path, _, _)| path == "link")
        .expect("link entry");
    assert!(link.1, "the link is stored as a link");
    assert!(link.2.is_empty(), "no passwd content");
}

#[test]
fn size_cap() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("big.bin");
    let file = fs::File::create(&path).expect("create");
    file.set_len(MAX_UPLOAD_BYTES + MIB).expect("sparse");

    let error = read_input(&path, false)
        .expect_err("over the cap")
        .to_string();

    assert!(error.contains("big.bin"), "{error}");
    assert!(error.contains("49 MB"), "{error}");
    assert!(error.contains("limit is 48 MB"), "{error}");
}

#[test]
fn text_and_binary_inputs() {
    let dir = tempfile::tempdir().expect("dir");
    let text = dir.path().join("a.txt");
    fs::write(&text, "a\tb").expect("write");
    let binary = dir.path().join("b.bin");
    fs::write(&binary, [0xff, 0xfe]).expect("write");

    assert_eq!(read_input(&text, true).expect("text"), json!("a\tb"));
    assert_eq!(
        decode(&read_input(&binary, false).expect("bytes")),
        [0xff, 0xfe]
    );

    let invalid = read_input(&binary, true)
        .expect_err("not utf-8")
        .to_string();
    assert!(invalid.contains("b.bin"), "{invalid}");
    let directory = read_input(dir.path(), true)
        .expect_err("directory")
        .to_string();
    assert!(
        directory.contains(&dir.path().display().to_string()),
        "{directory}"
    );
}

#[test]
fn write_output_replaces_atomically() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("out.bin");
    fs::write(&path, "old").expect("write");

    let written = write_output(&path, &json!(STANDARD.encode([1u8, 2, 3])), true).expect("write");

    assert_eq!(written, 3);
    assert_eq!(fs::read(&path).expect("read"), [1, 2, 3]);
    assert_eq!(
        fs::read_dir(dir.path()).expect("dir").count(),
        1,
        "no temp file left"
    );
}

#[test]
fn write_output_keeps_the_mode_and_follows_symlinks() {
    let dir = tempfile::tempdir().expect("dir");
    let real = dir.path().join("real.txt");
    fs::write(&real, "old").expect("write");
    fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).expect("chmod");
    let link = dir.path().join("link.txt");
    symlink(&real, &link).expect("symlink");

    write_output(&link, &json!("new"), false).expect("write");

    assert!(fs::symlink_metadata(&link).expect("meta").is_symlink());
    assert_eq!(fs::read_to_string(&real).expect("read"), "new");
    assert_eq!(
        fs::metadata(&real).expect("meta").permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn new_outputs_are_private() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("cookies.txt");

    write_output(&path, &json!("x"), false).expect("write");

    assert_eq!(
        fs::metadata(&path).expect("meta").permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn write_output_writes_devices_in_place() {
    write_output(Path::new("/dev/null"), &json!("x"), false).expect("write");

    assert!(!fs::metadata("/dev/null").expect("meta").is_file());
}

#[test]
fn pipes_are_read_under_the_cap() {
    let pipe = Path::new("/dev/zero");

    let error = read_input(pipe, false).expect_err("endless").to_string();

    assert!(error.contains("over 48 MB"), "{error}");
}

#[test]
fn tail_returns_the_last_lines_across_chunk_boundaries() {
    let dir = tempfile::tempdir().expect("dir");
    let log = dir.path().join("daemon.log");
    let total = TAIL_CHUNK as usize;
    let text: String = (0..total).map(|n| format!("line {n}\n")).collect();
    fs::write(&log, &text).expect("write");

    let tail = tail_lines(&log, 3).expect("tail");
    assert_eq!(
        tail,
        [
            format!("line {}", total - 3),
            format!("line {}", total - 2),
            format!("line {}", total - 1)
        ]
    );

    // More lines than the file has: all of them, none cut.
    assert_eq!(tail_lines(&log, total * 2).expect("all").len(), total);
}

#[test]
fn tail_keeps_a_last_line_without_a_newline() {
    let dir = tempfile::tempdir().expect("dir");
    let log = dir.path().join("daemon.log");
    fs::write(&log, "a\nb\nc").expect("write");

    assert_eq!(tail_lines(&log, 2).expect("tail"), ["b", "c"]);
}
