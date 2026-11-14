use std::fs;
use std::io::Write;
use std::os::unix::fs::{FileExt, symlink};
use std::path::Path;

use age::secrecy::SecretString;
use brr_core::{Error, Secret};

use super::{MANIFEST_NAME, read_archive, write_archive};

const PASSPHRASE: &str = "correct horse";
const MANIFEST: &[u8] = br#"{"name":"distinctive-profile-name"}"#;
const BUDGET: u64 = u64::MAX;
const SCRYPT_HEADER_PROBE_BYTES: usize = 200;
const BIG_LEN: usize = 300 * 1024;

fn passphrase() -> Secret {
    Secret::new(PASSPHRASE)
}

// Incompressible (xorshift64) so the stream spans several 64 KiB age chunks after zstd.
fn big_file() -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    (0..BIG_LEN)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

fn assert_invalid(result: Result<Vec<u8>, Error>, expected: &str) {
    match result {
        Err(Error::InvalidArgument { field, .. }) => assert_eq!(field, expected),
        other => panic!("expected InvalidArgument {expected}, got {other:?}"),
    }
}

fn sample_archive(root: &Path) -> std::path::PathBuf {
    let data = root.join("data");
    fs::create_dir_all(data.join("nested/deep")).expect("dirs");
    fs::write(data.join("big.bin"), big_file()).expect("big");
    fs::write(data.join("nested/deep/a.txt"), b"hello").expect("nested");
    let out = root.join("out.age");
    write_archive(&data, MANIFEST, &[], &passphrase(), &out).expect("write");
    out
}

#[test]
fn round_trip() {
    let root = tempfile::tempdir().expect("tempdir");
    let data = root.path().join("data");
    fs::create_dir_all(data.join("nested/deep")).expect("dirs");
    fs::create_dir_all(data.join("Cache")).expect("cache");
    fs::write(data.join("Cache/x"), b"cache").expect("cache file");
    fs::write(data.join("big.bin"), big_file()).expect("big");
    fs::write(data.join("nested/deep/a.txt"), b"hello").expect("nested");
    symlink("/nonexistent", data.join("SingletonLock")).expect("symlink");
    let out = root.path().join("out.age");

    let size = write_archive(
        &data,
        MANIFEST,
        &["Cache", "SingletonLock"],
        &passphrase(),
        &out,
    )
    .expect("write");
    assert_eq!(size, fs::metadata(&out).expect("meta").len());

    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let manifest = read_archive(&out, &passphrase(), &dest, BUDGET).expect("read");

    assert_eq!(manifest, MANIFEST);
    assert_eq!(fs::read(dest.join("big.bin")).expect("big"), big_file());
    assert_eq!(
        fs::read(dest.join("nested/deep/a.txt")).expect("a"),
        b"hello"
    );
    assert!(!dest.join("Cache").exists());
    assert!(dest.join("SingletonLock").symlink_metadata().is_err());
}

#[test]
fn wrong_passphrase() {
    let root = tempfile::tempdir().expect("tempdir");
    let out = sample_archive(root.path());
    let junk = root.path().join("junk");
    fs::write(&junk, (0..1024).map(|i| (i * 7) as u8).collect::<Vec<_>>()).expect("junk");
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");

    assert_invalid(
        read_archive(&out, &Secret::new("wrong"), &dest, BUDGET),
        "passphrase",
    );
    assert_invalid(
        read_archive(&junk, &passphrase(), &dest, BUDGET),
        "passphrase",
    );
    assert_eq!(fs::read_dir(&dest).expect("dest").count(), 0);
}

fn raw_entry(name: &[u8], kind: tar::EntryType, link: Option<&str>) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(kind);
    header.set_size(0);
    header.set_mode(0o644);
    if let Some(link) = link {
        header.set_link_name(link).expect("link");
    }
    header.as_old_mut().name[..name.len()].copy_from_slice(name);
    header.set_cksum();
    header
}

fn hand_built(path: &Path, second: tar::Header) {
    let file = fs::File::create(path).expect("create");
    let mut recipient = age::scrypt::Recipient::new(SecretString::from(PASSPHRASE));
    recipient.set_work_factor(super::ARCHIVE_WORK_FACTOR);
    let age = age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
        .expect("recipient")
        .wrap_output(file)
        .expect("wrap");
    let zstd = zstd::Encoder::new(age, 3).expect("zstd");
    let mut tar = tar::Builder::new(zstd);
    let mut first = tar::Header::new_gnu();
    first.set_size(MANIFEST.len() as u64);
    tar.append_data(&mut first, MANIFEST_NAME, MANIFEST)
        .expect("manifest");
    tar.append(&second, std::io::empty()).expect("second");
    let age = tar
        .into_inner()
        .expect("tar")
        .finish()
        .expect("zstd finish");
    age.finish().expect("age finish").flush().expect("flush");
}

#[test]
fn rejects_links_and_traversal() {
    let root = tempfile::tempdir().expect("tempdir");
    let dest = root.path().join("nest/dest");
    fs::create_dir_all(&dest).expect("dest");
    let outside = root.path().join("outside");
    fs::create_dir(&outside).expect("outside");
    let outside_name = outside.to_str().expect("utf8 path");
    let cases = [
        raw_entry(b"data/x", tar::EntryType::Symlink, Some(outside_name)),
        raw_entry(b"data/h", tar::EntryType::Link, Some("data/y")),
        raw_entry(b"data/../../evil", tar::EntryType::Regular, None),
        raw_entry(b"/evil", tar::EntryType::Regular, None),
        raw_entry(b"other/evil", tar::EntryType::Regular, None),
    ];

    for (i, header) in cases.into_iter().enumerate() {
        let path = root.path().join(format!("case{i}.age"));
        hand_built(&path, header);
        assert_invalid(read_archive(&path, &passphrase(), &dest, BUDGET), "file");
    }

    assert_eq!(fs::read_dir(&dest).expect("dest").count(), 0);
    // dest/../../evil is the escape target of the traversal entry.
    assert!(!root.path().join("evil").exists());
    assert_eq!(fs::read_dir(&outside).expect("outside").count(), 0);
}

#[test]
fn sparse_file_round_trips() {
    let root = tempfile::tempdir().expect("tempdir");
    let data = root.path().join("data");
    fs::create_dir(&data).expect("data");
    let sparse = fs::File::create(data.join("holes")).expect("create");
    sparse.set_len(4 * 1024 * 1024).expect("grow");
    sparse
        .write_all_at(b"tail", 4 * 1024 * 1024 - 4)
        .expect("tail");
    let out = root.path().join("out.age");
    write_archive(&data, MANIFEST, &[], &passphrase(), &out).expect("write");
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");

    read_archive(&out, &passphrase(), &dest, BUDGET).expect("read");

    assert_eq!(
        fs::read(dest.join("holes")).expect("holes"),
        fs::read(data.join("holes")).expect("source")
    );
}

#[test]
fn is_encrypted() {
    let root = tempfile::tempdir().expect("tempdir");
    let out = sample_archive(root.path());

    let bytes = fs::read(out).expect("read");

    assert!(bytes.starts_with(b"age-encryption.org/v1"));
    assert!(
        !bytes
            .windows(b"distinctive".len())
            .any(|w| w == b"distinctive")
    );
}

#[test]
fn extraction_stops_at_the_byte_budget() {
    let root = tempfile::tempdir().expect("tempdir");
    let out = sample_archive(root.path());
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");

    let result = read_archive(&out, &passphrase(), &dest, BIG_LEN as u64 / 2);

    assert!(
        matches!(&result, Err(Error::InvalidArgument { field, .. }) if field == "file"),
        "{result:?}"
    );
}

#[test]
fn export_pins_the_scrypt_cost() {
    let root = tempfile::tempdir().expect("tempdir");
    let out = sample_archive(root.path());

    let header = fs::read(&out).expect("read archive");
    let header = String::from_utf8_lossy(&header[..SCRYPT_HEADER_PROBE_BYTES]).into_owned();
    let stanza = header
        .lines()
        .find(|line| line.starts_with("-> scrypt"))
        .expect("scrypt stanza");

    assert_eq!(
        stanza.split(' ').next_back(),
        Some(super::ARCHIVE_WORK_FACTOR.to_string().as_str())
    );
}
