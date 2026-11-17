use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use brr_engine::sandbox::files::{self, COMPLETE_MARKER, PUT_DONE, Request};
use brr_proto::frames::{Link, Side};
use nix::unistd::Pid;
use tempfile::TempDir;
use test_case::test_case;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
use tokio::time::timeout;

use super::{
    Ending, FAILURE, IMAGE_HELP, SIGNAL_EXIT_BASE, exit_code, parse, serve_files, shut_down,
};

const SPEC_VERSION: &str = "1.2.3";
const OTHER_VERSION: &str = "9.9.9";
const INSTANCE_ID: &str = "01J9ZQ3V5X8K2M4N6P7R9T0W1Y";
const PROGRAM: &str = "/opt/chrome/chrome";
const SPEC_JSON: &str = r#"{
    "version": "1.2.3",
    "instance_id": "01J9ZQ3V5X8K2M4N6P7R9T0W1Y",
    "argv": ["/opt/chrome/chrome", "--remote-debugging-pipe"],
    "env": {"LANG": "en_US.UTF-8"},
    "xvfb": {"width": 1920, "height": 1080},
    "writable": ["/data/profiles/a", "/data/runs/x"],
    "wait_for_files": true,
    "copy_back": "/data/profiles/a"
}"#;

// Exits on SIGTERM; the loop keeps sh waking up to run the trap.
const EXITS_ON_TERM: &str = "trap 'exit 7' TERM; while :; do sleep 0.05; done";
const IGNORES_TERM: &str = "trap '' TERM; while :; do sleep 0.05; done";
// A browser that closed its pipe and still writes its profile for a moment.
const EXITS_SOON: &str = "sleep 0.3; exit 3";
const LONG_GRACE: Duration = Duration::from_secs(10);
const SHORT_GRACE: Duration = Duration::from_millis(200);
// Far below LONG_GRACE: the browser ended without the runner waiting the grace out.
const PROMPT: Duration = Duration::from_secs(5);
const KILLED: i32 = SIGNAL_EXIT_BASE + 9;
const TRANSPORT_BYTES: usize = 64 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);
const COOKIES: &str = "Cookies";
const CONTENT: &str = "cookie jar";
const ESCAPE: &str = "escape";

fn spawn(script: &str) -> (Child, Option<Pid>) {
    let child = Command::new("sh")
        .args(["-c", script])
        .process_group(0)
        .spawn()
        .expect("sh starts");
    let group = child
        .id()
        .and_then(|pid| i32::try_from(pid).ok())
        .map(Pid::from_raw);

    (child, group)
}

#[test]
fn parse_rejects_other_version() {
    let error = parse(SPEC_JSON.as_bytes(), OTHER_VERSION)
        .err()
        .map(|error| format!("{error:#}"));

    assert_eq!(
        error,
        Some(format!(
            "runner version {OTHER_VERSION} does not match brr {SPEC_VERSION}; {IMAGE_HELP}"
        ))
    );
}

#[test]
fn parse_round_trips() {
    let spec = parse(SPEC_JSON.as_bytes(), SPEC_VERSION).expect("the fixture parses");

    assert_eq!(spec.instance_id.to_string(), INSTANCE_ID);
    assert_eq!(spec.argv.first().map(String::as_str), Some(PROGRAM));

    let json = serde_json::to_vec(&spec).expect("the spec serializes");
    assert_eq!(
        parse(&json, SPEC_VERSION).expect("the spec parses again"),
        spec
    );
}

#[test_case("" ; "empty")]
#[test_case("{}" ; "missing fields")]
fn parse_rejects_malformed(json: &str) {
    assert!(parse(json.as_bytes(), SPEC_VERSION).is_err());
}

#[test_case(Ok(ExitStatus::from_raw(0)), 0 ; "clean exit")]
#[test_case(Ok(ExitStatus::from_raw(3 << 8)), 3 ; "exit code")]
#[test_case(Ok(ExitStatus::from_raw(9)), KILLED ; "signal")]
#[test_case(Err(std::io::Error::other("gone")), FAILURE ; "wait failed")]
fn exit_code_maps_status(status: std::io::Result<ExitStatus>, expected: i32) {
    assert_eq!(exit_code(status), expected);
}

#[test_case(Ending::Stop, EXITS_ON_TERM, LONG_GRACE, 0 ; "stop sends SIGTERM")]
#[test_case(Ending::Stop, IGNORES_TERM, SHORT_GRACE, 0 ; "stop kills after the grace")]
#[test_case(Ending::Closed, EXITS_SOON, LONG_GRACE, 3 ; "closed pipe waits for the exit")]
#[test_case(Ending::Closed, IGNORES_TERM, SHORT_GRACE, KILLED ; "closed pipe kills after the grace")]
#[tokio::test]
async fn shut_down_gives_the_browser_its_grace(
    ending: Ending,
    script: &str,
    grace: Duration,
    expected: i32,
) {
    let (mut child, group) = spawn(script);
    let started = Instant::now();

    let code = shut_down(&mut child, group, ending, grace).await;

    assert_eq!(code, expected);
    assert!(started.elapsed() < PROMPT);
}

// brr's end of a runner that serves files for one writable folder inside `dir`.
struct Served {
    dir: TempDir,
    writable: PathBuf,
    link: Link,
    started: oneshot::Receiver<()>,
}

fn served() -> Served {
    let dir = TempDir::new().unwrap();
    let writable = dir.path().join("run");
    std::fs::create_dir_all(&writable).unwrap();
    let (daemon, runner) = tokio::io::duplex(TRANSPORT_BYTES);
    let (reader, writer) = tokio::io::split(daemon);
    let (link, _, _) = Link::start(reader, writer, Side::Daemon);
    let (reader, writer) = tokio::io::split(runner);
    let (_, _, accepted) = Link::start(reader, writer, Side::Runner);
    let (started_tx, started) = oneshot::channel();
    tokio::spawn(serve_files(accepted, vec![writable.clone()], started_tx));

    Served {
        dir,
        writable,
        link,
        started,
    }
}

fn open(link: &Link, request: &Request) -> brr_proto::frames::Stream {
    link.open(&serde_json::to_vec(request).unwrap()).unwrap()
}

async fn put(link: &Link, dir: &Path, archive: &[u8]) -> String {
    let mut stream = open(link, &Request::Put(dir.to_owned()));
    stream.write_all(archive).await.unwrap();
    stream.shutdown().await.unwrap();
    let mut answer = String::new();
    timeout(TIMEOUT, stream.read_to_string(&mut answer))
        .await
        .unwrap()
        .unwrap();

    answer
}

async fn profile_archive() -> Vec<u8> {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join(COOKIES), CONTENT).unwrap();

    files::pack(dir.path().to_owned(), Vec::new())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_put_lands_in_the_run_before_the_start() {
    let mut served = served();
    let into = served.writable.join("profile");

    let answer = put(&served.link, &into, &profile_archive().await).await;
    let early = served.started.try_recv();
    drop(open(&served.link, &Request::Start));

    assert_eq!(answer, PUT_DONE);
    assert_eq!(
        std::fs::read_to_string(into.join(COOKIES)).unwrap(),
        CONTENT
    );
    assert!(early.is_err());
    timeout(TIMEOUT, served.started).await.unwrap().unwrap();
}

#[test_case(|writable: &Path| writable.join("..").join(ESCAPE); "parent components")]
#[test_case(|writable: &Path| writable.parent().unwrap().join(ESCAPE); "outside the run")]
#[test_case(|_: &Path| PathBuf::from(ESCAPE); "relative")]
#[tokio::test]
async fn a_put_outside_the_run_is_refused(dir: fn(&Path) -> PathBuf) {
    let served = served();

    let answer = put(
        &served.link,
        &dir(&served.writable),
        &profile_archive().await,
    )
    .await;

    assert_ne!(answer, PUT_DONE);
    assert!(!served.dir.path().join(ESCAPE).exists());
}

#[tokio::test]
async fn a_put_with_a_link_is_refused() {
    let served = served();
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    builder
        .append_link(&mut header, ESCAPE, "/etc/passwd")
        .unwrap();
    let mut marker = tar::Header::new_gnu();
    marker.set_size(0);
    builder
        .append_data(&mut marker, COMPLETE_MARKER, std::io::empty())
        .unwrap();
    let archive = builder.into_inner().unwrap();

    let answer = put(&served.link, &served.writable, &archive).await;

    assert_ne!(answer, PUT_DONE);
    assert!(served.writable.join(ESCAPE).symlink_metadata().is_err());
}

async fn get(link: &Link, path: &Path) -> std::io::Result<Vec<u8>> {
    let mut stream = open(link, &Request::Get(path.to_owned()));
    stream.shutdown().await.unwrap();
    let size = timeout(TIMEOUT, stream.read_u64()).await.unwrap()?;
    let mut bytes = Vec::new();
    (&mut stream).take(size).read_to_end(&mut bytes).await?;

    Ok(bytes)
}

#[tokio::test]
async fn a_get_sends_a_file_of_the_run() {
    let served = served();
    let path = served.writable.join(COOKIES);
    std::fs::write(&path, CONTENT).unwrap();

    let bytes = get(&served.link, &path).await.unwrap();

    assert_eq!(bytes, CONTENT.as_bytes());
}

#[test_case(|served: &Served| {
    let path = served.dir.path().join(COOKIES);
    std::fs::write(&path, CONTENT).unwrap();
    path
}; "outside the run")]
#[test_case(|served: &Served| {
    let outside = served.dir.path().join(COOKIES);
    std::fs::write(&outside, CONTENT).unwrap();
    let link = served.writable.join(ESCAPE);
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    link
}; "a link in the run")]
#[test_case(|served: &Served| served.writable.join("..").join(COOKIES); "parent components")]
#[tokio::test]
async fn a_get_of_anything_else_is_refused(path: fn(&Served) -> PathBuf) {
    let served = served();
    let path = path(&served);

    let error = get(&served.link, &path).await.unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
}
