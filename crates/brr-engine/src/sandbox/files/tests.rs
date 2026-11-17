use std::io::{Cursor, ErrorKind};
use std::path::{Path, PathBuf};

use tar::{Builder, EntryType, Header};
use tempfile::TempDir;
use test_case::test_case;

use super::{COMPLETE_MARKER, Request, inside, pack, unpack};

const RUN_DIR: &str = "/data/runs/x";
const PROFILE_DIR: &str = "/data/profiles/a";
const NESTED: &str = "Default/Cookies";
const TOP: &str = "Local State";
const EMPTY_DIR: &str = "Cache";
const CONTENT: &str = "cookie jar";
const LINK: &str = "SingletonLock";

fn dirs() -> Vec<PathBuf> {
    vec![PathBuf::from(RUN_DIR), PathBuf::from(PROFILE_DIR)]
}

#[test_case("/data/runs/x", true; "the folder itself")]
#[test_case("/data/profiles/a/Default", true; "below a folder")]
#[test_case("/data/profiles/b", false; "a sibling")]
#[test_case("/data/runs/x/../../profiles/b", false; "parent components")]
#[test_case("/data/runs/x/./y", true; "current components fold away")]
#[test_case("data/runs/x", false; "relative")]
#[test_case("/etc/passwd", false; "elsewhere")]
fn a_path_is_inside_only_below_a_folder(path: &str, expected: bool) {
    assert_eq!(inside(Path::new(path), &dirs()), expected);
}

#[test_case(Request::Spec, r#""spec""#; "spec")]
#[test_case(Request::Put(PathBuf::from(RUN_DIR)), r#"{"put":"/data/runs/x"}"#; "put")]
#[test_case(Request::Get(PathBuf::from(RUN_DIR)), r#"{"get":"/data/runs/x"}"#; "get")]
#[test_case(Request::Start, r#""start""#; "start")]
#[test_case(Request::Profile, r#""profile""#; "profile")]
fn a_request_is_its_json(request: Request, json: &str) {
    assert_eq!(serde_json::to_string(&request).unwrap(), json);
    assert_eq!(serde_json::from_str::<Request>(json).unwrap(), request);
}

fn source() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("Default")).unwrap();
    std::fs::create_dir_all(dir.path().join(EMPTY_DIR)).unwrap();
    std::fs::write(dir.path().join(NESTED), CONTENT).unwrap();
    std::fs::write(dir.path().join(TOP), CONTENT).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", dir.path().join(LINK)).unwrap();

    dir
}

async fn packed(dir: &Path) -> Vec<u8> {
    pack(dir.to_owned(), Vec::new()).await.unwrap()
}

#[tokio::test]
async fn a_folder_round_trips_without_its_links() {
    let source = source();
    let dest = TempDir::new().unwrap();
    let into = dest.path().join("profile");

    unpack(Cursor::new(packed(source.path()).await), into.clone())
        .await
        .unwrap();

    assert_eq!(std::fs::read_to_string(into.join(NESTED)).unwrap(), CONTENT);
    assert_eq!(std::fs::read_to_string(into.join(TOP)).unwrap(), CONTENT);
    assert!(into.join(EMPTY_DIR).is_dir());
    assert!(into.join(LINK).symlink_metadata().is_err());
    assert!(!into.join(COMPLETE_MARKER).exists());
}

#[tokio::test]
async fn an_archive_cut_short_is_refused() {
    let source = source();
    let mut bytes = packed(source.path()).await;
    // The marker entry and the end blocks: what is left ends cleanly after the last real entry.
    bytes.truncate(bytes.len() - 3 * 512);
    let dest = TempDir::new().unwrap();

    let error = unpack(Cursor::new(bytes), dest.path().join("profile"))
        .await
        .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
}

fn archive(build: impl FnOnce(&mut Builder<Vec<u8>>)) -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());
    build(&mut builder);
    let mut marker = Header::new_gnu();
    marker.set_entry_type(EntryType::Regular);
    marker.set_size(0);
    builder
        .append_data(&mut marker, COMPLETE_MARKER, std::io::empty())
        .unwrap();

    builder.into_inner().unwrap()
}

// set_path refuses `..` and absolute names, so these are written into the header directly.
fn raw_file(builder: &mut Builder<Vec<u8>>, name: &str) {
    let mut header = Header::new_old();
    header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
    header.set_entry_type(EntryType::Regular);
    header.set_size(CONTENT.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    builder.append(&header, CONTENT.as_bytes()).unwrap();
}

fn link(builder: &mut Builder<Vec<u8>>, kind: EntryType) {
    let mut header = Header::new_gnu();
    header.set_entry_type(kind);
    header.set_size(0);
    builder
        .append_link(&mut header, "escape", "/etc/passwd")
        .unwrap();
}

#[test_case(archive(|builder| raw_file(builder, "../escape")); "parent path")]
#[test_case(archive(|builder| raw_file(builder, "/tmp/escape")); "absolute path")]
#[test_case(archive(|builder| link(builder, EntryType::Symlink)); "symlink")]
#[test_case(archive(|builder| link(builder, EntryType::Link)); "hardlink")]
#[test_case({
    let mut bytes = archive(|_| {});
    bytes.truncate(512);
    let mut builder = Builder::new(bytes);
    raw_file(&mut builder, "late");
    builder.into_inner().unwrap()
}; "an entry after the marker")]
#[tokio::test]
async fn a_hostile_archive_is_refused(bytes: Vec<u8>) {
    let dest = TempDir::new().unwrap();
    let into = dest.path().join("profile");

    let error = unpack(Cursor::new(bytes), into.clone()).await.unwrap_err();

    assert_eq!(error.kind(), ErrorKind::InvalidData, "{error}");
    assert!(!dest.path().join("escape").exists());
    assert!(!into.join("escape").exists());
}
