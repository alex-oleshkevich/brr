use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use tokio::sync::mpsc;

use brr_core::{BuildSource, Error, InstallPhase, sha256_hex};

use super::{Progress, download};
use crate::{
    Release, install,
    test_support::{serve, zip_bytes},
};

const ARCHIVE_PATH: &str = "/chrome.zip";
const CHROME_ENTRY: &str = "chrome-linux64/chrome";
const CHROME_MODE: u32 = 0o755;
const VERSION: &str = "152.0.7444.59";
const PROGRESS_CAPACITY: usize = 64;

#[derive(Clone, Copy)]
enum RangeBehavior {
    Honor,
    Ignore,
    Reject,
    Complete,
}

struct Archive {
    body: Vec<u8>,
    behavior: RangeBehavior,
    ranges: Mutex<Vec<Option<String>>>,
}

async fn archive(State(archive): State<Arc<Archive>>, headers: HeaderMap) -> Response {
    let range = headers
        .get(header::RANGE)
        .map(|value| value.to_str().unwrap().to_owned());
    archive.ranges.lock().unwrap().push(range.clone());

    let Some(range) = range else {
        return archive.body.clone().into_response();
    };
    match archive.behavior {
        RangeBehavior::Ignore => archive.body.clone().into_response(),
        RangeBehavior::Reject => StatusCode::RANGE_NOT_SATISFIABLE.into_response(),
        RangeBehavior::Honor => {
            let start: usize = range
                .strip_prefix("bytes=")
                .and_then(|rest| rest.strip_suffix('-'))
                .unwrap()
                .parse()
                .unwrap();
            (StatusCode::PARTIAL_CONTENT, archive.body[start..].to_vec()).into_response()
        }
        RangeBehavior::Complete => (
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(
                header::CONTENT_RANGE,
                format!("bytes */{}", archive.body.len()),
            )],
        )
            .into_response(),
    }
}

async fn serve_archive(body: Vec<u8>, behavior: RangeBehavior) -> (String, Arc<Archive>) {
    let archive_state = Arc::new(Archive {
        body,
        behavior,
        ranges: Mutex::new(Vec::new()),
    });
    let base = serve(
        Router::new()
            .route(ARCHIVE_PATH, get(archive))
            .with_state(archive_state.clone()),
    )
    .await;

    (format!("{base}{ARCHIVE_PATH}"), archive_state)
}

fn chrome_zip() -> Vec<u8> {
    zip_bytes(&[
        (CHROME_ENTRY, &[7; 4096], CHROME_MODE),
        ("locales/en-US.pak", &[1; 2048], 0o644),
    ])
}

fn drain(mut receiver: mpsc::Receiver<Progress>) -> Vec<Progress> {
    let mut events = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        events.push(event);
    }

    events
}

#[tokio::test]
async fn resumes_from_part() {
    let zip = chrome_zip();
    let (url, state) = serve_archive(zip.clone(), RangeBehavior::Honor).await;
    let builds = tempfile::tempdir().unwrap();
    let cft = builds.path().join("cft");
    fs::create_dir_all(&cft).unwrap();
    let half = zip.len() / 2;
    fs::write(cft.join(format!("{VERSION}.zip.part")), &zip[..half]).unwrap();
    let release = Release {
        source: BuildSource::Cft,
        version: VERSION.into(),
        channel: None,
        url,
        sha256: Some(sha256_hex(&zip)),
    };
    let (sender, receiver) = mpsc::channel(PROGRESS_CAPACITY);

    let installed = install(&reqwest::Client::new(), &release, builds.path(), sender)
        .await
        .unwrap();

    assert_eq!(
        *state.ranges.lock().unwrap(),
        [Some(format!("bytes={half}-"))]
    );
    assert_eq!(installed.sha256, Some(sha256_hex(&zip)));
    assert_eq!(installed.size_bytes, Some(zip.len() as u64));
    assert_eq!(installed.executable, cft.join(VERSION).join(CHROME_ENTRY));
    let mode = fs::metadata(&installed.executable)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, CHROME_MODE);
    assert!(!cft.join(format!("{VERSION}.zip.part")).exists());
    assert!(!cft.join(format!("{VERSION}.tmp")).exists());
    assert_eq!(
        drain(receiver).last(),
        Some(&Progress {
            bytes: zip.len() as u64,
            total: Some(zip.len() as u64),
            phase: InstallPhase::Extracting
        })
    );
}

#[tokio::test]
async fn existing_install_is_returned_without_a_request() {
    let builds = tempfile::tempdir().unwrap();
    let executable = builds.path().join("cft").join(VERSION).join(CHROME_ENTRY);
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::write(&executable, b"chrome").unwrap();
    let release = Release {
        source: BuildSource::Cft,
        version: VERSION.into(),
        channel: None,
        url: "http://127.0.0.1:1/unreachable.zip".into(),
        sha256: None,
    };
    let (sender, _receiver) = mpsc::channel(PROGRESS_CAPACITY);

    let installed = install(&reqwest::Client::new(), &release, builds.path(), sender)
        .await
        .unwrap();

    assert_eq!(installed.executable, executable);
    assert_eq!(installed.sha256, None);
    assert_eq!(installed.size_bytes, None);
}

#[tokio::test]
async fn restarts_when_server_ignores_range() {
    let body = vec![3; 5000];
    let (url, state) = serve_archive(body.clone(), RangeBehavior::Ignore).await;
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("a.part");
    fs::write(&part, &body[..100]).unwrap();
    let (sender, _receiver) = mpsc::channel(PROGRESS_CAPACITY);
    let expected = sha256_hex(&body);

    let downloaded = download(
        &reqwest::Client::new(),
        &url,
        &part,
        Some(&expected),
        Some(sender),
    )
    .await
    .unwrap();

    assert_eq!(state.ranges.lock().unwrap().len(), 1);
    assert_eq!(downloaded.sha256, expected);
    assert_eq!(fs::read(&part).unwrap(), body);
}

#[tokio::test]
async fn restarts_once_when_range_is_unsatisfiable() {
    let body = vec![5; 5000];
    let (url, state) = serve_archive(body.clone(), RangeBehavior::Reject).await;
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("a.part");
    fs::write(&part, vec![9; 100]).unwrap();
    let (sender, _receiver) = mpsc::channel(PROGRESS_CAPACITY);
    let expected = sha256_hex(&body);

    let downloaded = download(
        &reqwest::Client::new(),
        &url,
        &part,
        Some(&expected),
        Some(sender),
    )
    .await
    .unwrap();

    assert_eq!(
        *state.ranges.lock().unwrap(),
        [Some("bytes=100-".to_owned()), None]
    );
    assert_eq!(downloaded.sha256, expected);
}

#[tokio::test]
async fn complete_part_is_reused() {
    let body = vec![6; 5000];
    let (url, state) = serve_archive(body.clone(), RangeBehavior::Complete).await;
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("a.part");
    fs::write(&part, &body).unwrap();
    let expected = sha256_hex(&body);

    let downloaded = download(&reqwest::Client::new(), &url, &part, Some(&expected), None)
        .await
        .unwrap();

    assert_eq!(
        *state.ranges.lock().unwrap(),
        [Some(format!("bytes={}-", body.len()))]
    );
    assert_eq!(downloaded.sha256, expected);
    assert_eq!(downloaded.size_bytes, body.len() as u64);
    assert_eq!(fs::read(&part).unwrap(), body);
}

#[tokio::test]
async fn part_is_not_resumed_without_a_checksum() {
    let body = vec![8; 5000];
    let (url, state) = serve_archive(body.clone(), RangeBehavior::Honor).await;
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("a.part");
    fs::write(&part, vec![9; 100]).unwrap();

    let downloaded = download(&reqwest::Client::new(), &url, &part, None, None)
        .await
        .unwrap();

    assert_eq!(*state.ranges.lock().unwrap(), [None]);
    assert_eq!(downloaded.sha256, sha256_hex(&body));
    assert_eq!(fs::read(&part).unwrap(), body);
}

#[tokio::test]
async fn progress_steps_by_megabyte_and_ends_at_total() {
    let body = vec![1; 2_500_000];
    let (url, _state) = serve_archive(body.clone(), RangeBehavior::Honor).await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, receiver) = mpsc::channel(PROGRESS_CAPACITY);

    download(
        &reqwest::Client::new(),
        &url,
        &dir.path().join("a.part"),
        None,
        Some(sender),
    )
    .await
    .unwrap();

    let events = drain(receiver);
    assert!(events.len() >= 2);
    assert!(
        events
            .iter()
            .all(|event| event.total == Some(body.len() as u64))
    );
    assert_eq!(events.last().unwrap().bytes, body.len() as u64);
}

#[tokio::test]
async fn http_error_is_reported() {
    let base = serve(Router::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let (sender, _receiver) = mpsc::channel(PROGRESS_CAPACITY);

    let result = download(
        &reqwest::Client::new(),
        &format!("{base}/missing"),
        &dir.path().join("a.part"),
        None,
        Some(sender),
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn checksum_verified() {
    let zip = chrome_zip();
    let (url, _state) = serve_archive(zip.clone(), RangeBehavior::Honor).await;
    let builds = tempfile::tempdir().unwrap();
    let http = reqwest::Client::new();
    let wrong = "0".repeat(64);
    let mut release = Release {
        source: BuildSource::Cft,
        version: VERSION.into(),
        channel: None,
        url: url.clone(),
        sha256: Some(wrong.clone()),
    };
    let part = builds
        .path()
        .join("cft")
        .join(format!("{VERSION}.zip.part"));

    let (sender, _receiver) = mpsc::channel(PROGRESS_CAPACITY);
    let failed = install(&http, &release, builds.path(), sender).await;

    let Err(Error::Internal { message }) = failed else {
        panic!("expected a checksum failure, got {failed:?}");
    };
    assert_eq!(
        message,
        format!(
            "checksum mismatch for {url}: expected {wrong}, got {}",
            sha256_hex(&zip)
        )
    );
    assert!(!part.exists());
    assert!(!builds.path().join("cft").join(VERSION).exists());

    release.sha256 = Some(sha256_hex(&zip).to_uppercase());
    let (sender, _receiver) = mpsc::channel(PROGRESS_CAPACITY);
    let installed = install(&http, &release, builds.path(), sender)
        .await
        .unwrap();

    assert_eq!(
        installed.executable,
        builds.path().join("cft").join(VERSION).join(CHROME_ENTRY)
    );
}
