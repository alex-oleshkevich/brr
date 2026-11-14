#![allow(clippy::unwrap_used)]

use crate::common;

use std::{fs, sync::Arc, time::Duration};

use axum::{
    Router,
    http::header::{CONTENT_DISPOSITION, CONTENT_TYPE},
    routing::get,
};
use brr_core::{
    AdmissionMode, ArtifactKind, BuildChoice, Display, Error, InstanceId, Order, Page, ProfileKind,
    ProfileSettings, QuotaScope, WaitUntil, capture::ReturnMode,
};
use brr_daemon::{
    App,
    services::{
        artifacts::{self, ArtifactOwner},
        instances::LaunchRequest,
        quota,
    },
};
use brr_store::{artifact, event, profile};
use serde_json::{Value, json};

use common::{
    KillOnDrop, boot, events_of, listen, profile_row, register_build, scratch_app, wait_for,
};

const KIB: usize = 1024;
const MIB: usize = 1024 * KIB;
const MIME: &str = "image/png";
const PUBLIC_PREFIX: &str = "http://127.0.0.1:9898/v1/a/";
const PUBLIC_ID_CHARS: usize = 32;
const DOWNLOAD_BYTES: usize = 3_000_000;
const DOWNLOAD_PATH: &str = "/big.bin";
const FLOOR_ENV: &str = "BRR_RETENTION__FREE_DISK_FLOOR_GB";
const NO_FLOOR: &str = "0";
const UNREACHABLE_FLOOR_GB: &str = "1000000000";
const FLOOR_BYTES: u64 = 1_000_000_000 * 1024 * 1024 * 1024;
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(10);

async fn events_by_kind(app: &Arc<App>, kind: &str) -> Vec<Value> {
    let query = event::Query {
        since: None,
        kinds: vec![kind.to_owned()],
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: Order::Asc,
        page: Page::new(Some(1000), None),
    };
    let (events, _) = app
        .store
        .run(move |c| event::list(c, &query))
        .await
        .unwrap();

    events.into_iter().map(|event| event.data).collect()
}

async fn artifacts_of(app: &Arc<App>, kind: ArtifactKind) -> usize {
    let filter = artifact::Filter {
        kind: Some(kind),
        ..artifact::Filter::default()
    };
    let (rows, _) = app
        .store
        .run(move |c| artifact::list(c, &filter, &Page::new(None, None)))
        .await
        .unwrap();

    rows.len()
}

#[tokio::test]
async fn deliver_modes() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let owner = ArtifactOwner::default;
    let small = vec![1; KIB];

    let inline = artifacts::deliver(
        &app,
        ArtifactKind::Screenshot,
        owner(),
        "a.png",
        MIME,
        small.clone(),
        ReturnMode::Auto,
    )
    .await
    .unwrap();
    assert_eq!(inline.inline_base64.unwrap().0, small);
    assert!(inline.artifact_id.is_none() && inline.url.is_none());
    assert_eq!(artifacts_of(&app, ArtifactKind::Screenshot).await, 0);

    let big = vec![2; 6 * MIB];
    let stored = artifacts::deliver(
        &app,
        ArtifactKind::Screenshot,
        owner(),
        "big one.png",
        MIME,
        big.clone(),
        ReturnMode::Auto,
    )
    .await
    .unwrap();
    let artifact_id = stored.artifact_id.unwrap();
    let url = stored.url.unwrap();
    let public_id = url.strip_prefix(PUBLIC_PREFIX).unwrap();
    assert_eq!(public_id.len(), PUBLIC_ID_CHARS);
    assert!(
        public_id
            .chars()
            .all(|c| matches!(c, '0'..='9' | 'a'..='f'))
    );
    assert!(stored.inline_base64.is_none());
    assert_eq!(artifacts_of(&app, ArtifactKind::Screenshot).await, 1);
    let created = events_by_kind(&app, "artifact.created").await;
    assert_eq!(created[0]["artifact_id"], json!(artifact_id));

    let response = reqwest::get(format!("http://{addr}/v1/a/{public_id}"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()[CONTENT_TYPE], MIME);
    assert!(
        response.headers()[CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("big_one.png")
    );
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["content-security-policy"], "sandbox");
    assert!(
        response.headers()[CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("inline")
    );
    assert_eq!(response.bytes().await.unwrap().as_ref(), big.as_slice());
    let by_id = reqwest::get(format!("http://{addr}/v1/artifacts/{artifact_id}/content"))
        .await
        .unwrap();
    assert_eq!(by_id.bytes().await.unwrap().len(), big.len());

    let both = artifacts::deliver(
        &app,
        ArtifactKind::Pdf,
        owner(),
        "a.pdf",
        MIME,
        small.clone(),
        ReturnMode::Both,
    )
    .await
    .unwrap();
    assert!(both.url.is_some() && both.artifact_id.is_some());
    assert_eq!(both.inline_base64.unwrap().0, small);

    // A page chose this file: it must not run as a document on the daemon's origin.
    let page = artifacts::store_bytes(
        &app,
        ArtifactKind::Download,
        owner(),
        "x.html",
        "text/html",
        b"<script>1</script>".to_vec().into(),
    )
    .await
    .unwrap();
    let served = reqwest::get(format!("http://{addr}/v1/a/{}", page.public_id))
        .await
        .unwrap();
    assert!(
        served.headers()[CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );

    let stored = app
        .store
        .run(move |c| artifact::get(c, &artifact_id))
        .await
        .unwrap();
    artifacts::delete(&app, &stored, "deleted").await.unwrap();
    let gone = reqwest::get(format!("http://{addr}/v1/a/{public_id}"))
        .await
        .unwrap();
    assert_eq!(gone.status(), 404);
    assert_eq!(events_by_kind(&app, "artifact.deleted").await.len(), 1);
}

#[tokio::test]
async fn quota_checks() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[(FLOOR_ENV, NO_FLOOR)]))
        .await
        .unwrap();
    let data_dir = dir.path().join("profile-data");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(data_dir.join("file"), [0; KIB]).unwrap();

    let over = profile_row(ProfileKind::Persistent, Some(data_dir.clone()), Some(1));
    assert_eq!(
        quota::check_launch(&app, &over).await,
        Err(Error::QuotaExceeded {
            scope: QuotaScope::Profile,
            used_bytes: KIB as u64,
            limit_bytes: 1,
        })
    );
    let within = profile_row(ProfileKind::Persistent, Some(data_dir), Some(MIB as u64));
    quota::check_launch(&app, &within).await.unwrap();
    let unlimited = profile_row(ProfileKind::Ephemeral, None, None);
    quota::check_launch(&app, &unlimited).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[(FLOOR_ENV, UNREACHABLE_FLOOR_GB)]))
        .await
        .unwrap();
    assert!(matches!(
        quota::check_launch(&app, &unlimited).await,
        Err(Error::QuotaExceeded {
            scope: QuotaScope::DiskFloor,
            limit_bytes: FLOOR_BYTES,
            ..
        })
    ));
    // The floor also stops storing a local Artifact.
    let stored = artifacts::store_bytes(
        &app,
        ArtifactKind::Pdf,
        ArtifactOwner::default(),
        "a.pdf",
        MIME,
        vec![0; KIB].into(),
    )
    .await;
    assert!(matches!(
        stored,
        Err(Error::QuotaExceeded {
            scope: QuotaScope::DiskFloor,
            ..
        })
    ));
}

#[tokio::test]
async fn download_limits() {
    let Some(chrome) = common::chrome() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[])).await.unwrap();
    let build_id = register_build(&app, &chrome).await;
    let settings = ProfileSettings {
        build: BuildChoice::BuildId(build_id),
        display: Display::Headless,
        ..ProfileSettings::default()
    };
    let mut found = profile_row(ProfileKind::Ephemeral, None, None);
    found.settings = settings;
    let row = found.clone();
    app.store
        .run(move |c| profile::insert(c, &row))
        .await
        .unwrap();

    let served = Router::new().route(
        DOWNLOAD_PATH,
        get(|| async {
            (
                [
                    (CONTENT_TYPE, "application/octet-stream"),
                    (CONTENT_DISPOSITION, "attachment; filename=\"big.bin\""),
                ],
                vec![7_u8; DOWNLOAD_BYTES],
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}{DOWNLOAD_PATH}", listener.local_addr().unwrap());
    tokio::spawn(std::future::IntoFuture::into_future(axum::serve(
        listener, served,
    )));

    let guard = KillOnDrop::new(&app, Vec::new());
    let (instance, tab) = app
        .instances
        .launch(
            &app,
            LaunchRequest {
                profile: found,
                display: None,
                priority: 5,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap();
    guard.track(instance.id);
    let instance_id: InstanceId = instance.id;

    app.set_settings([("retention.download_max_mb".to_owned(), json!(1))].into())
        .await
        .unwrap();
    // A download aborts the navigation that started it.
    let _ = app
        .instances
        .navigate(
            &instance_id,
            &tab.id,
            url.clone(),
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await;
    wait_for("download.cancelled", async || {
        !events_of(&app, "download.cancelled", instance_id)
            .await
            .is_empty()
    })
    .await;
    let cancelled = events_of(&app, "download.cancelled", instance_id).await;
    assert_eq!(cancelled[0]["reason"], "too_large");
    assert_eq!(artifacts_of(&app, ArtifactKind::Download).await, 0);

    app.set_settings([("retention.download_max_mb".to_owned(), json!(10))].into())
        .await
        .unwrap();
    let _ = app
        .instances
        .navigate(
            &instance_id,
            &tab.id,
            url,
            WaitUntil::Load,
            NAVIGATE_TIMEOUT,
        )
        .await;
    wait_for("download.completed", async || {
        !events_of(&app, "download.completed", instance_id)
            .await
            .is_empty()
    })
    .await;
    let completed = events_of(&app, "download.completed", instance_id).await;
    let artifact_id = serde_json::from_value(completed[0]["artifact_id"].clone()).unwrap();
    let stored = app
        .store
        .run(move |c| artifact::get(c, &artifact_id))
        .await
        .unwrap();
    assert_eq!(stored.kind, ArtifactKind::Download);
    assert_eq!(stored.size_bytes, DOWNLOAD_BYTES as u64);
}
