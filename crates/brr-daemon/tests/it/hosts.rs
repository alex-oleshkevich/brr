#![allow(clippy::unwrap_used)]

use crate::common;

use std::sync::Arc;

use brr_core::{
    AdmissionMode, ConflictReason, Display, Engine, Error, Host, InstanceId, SandboxMode, now_ms,
};
use brr_daemon::{App, ops, services::admission::AdmitRequest};
use serde_json::{Value, json};
use tokio::sync::watch;

use common::{boot, ctx, scratch_app};

const NAME: &str = "box-1";
// Nothing listens on port 1: the probe fails fast and the Host is still saved.
const UNREACHABLE_URL: &str = "ssh://alex@127.0.0.1:1";
const MAX_INSTANCES: u32 = 2;
const DOCKER_SANDBOX: (&str, &str) = ("BRR_SANDBOX__MODE", "docker");
const ONE_INSTANCE: (&str, &str) = ("BRR_LIMITS__MAX_INSTANCES", "1");

async fn admit(app: &Arc<App>, mode: AdmissionMode) -> Result<Option<Host>, Error> {
    let request = AdmitRequest {
        sandbox: SandboxMode::Docker,
        instance_id: InstanceId::new(),
        profile_name: "placed".to_owned(),
        engine: Engine::Chromium,
        display: Display::Headless,
        tags: Vec::new(),
        priority: 0,
        mode,
        queued_at: now_ms(),
    };

    app.admission
        .admit(app, request, &mut watch::channel(None).1)
        .await
}

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    let op = ops::find(name).unwrap();

    ops::dispatch(ctx(app), op, input).await
}

async fn create(
    app: &Arc<App>,
    name: &str,
    docker_url: &str,
    max_instances: u32,
) -> Result<Value, Error> {
    call(
        app,
        "host.create",
        json!({ "name": name, "docker_url": docker_url, "max_instances": max_instances }),
    )
    .await
}

fn invalid_field(result: Result<Value, Error>) -> String {
    match result {
        Err(Error::InvalidArgument { field, .. }) => field,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[tokio::test]
async fn unreachable_host_is_saved_with_its_error() {
    let (_dir, app) = scratch_app().await;

    let created: Host = serde_json::from_value(
        create(&app, NAME, UNREACHABLE_URL, MAX_INSTANCES)
            .await
            .unwrap(),
    )
    .unwrap();
    let listed = call(&app, "host.list", json!({})).await.unwrap();
    let hosts: Vec<Host> = serde_json::from_value(listed["hosts"].clone()).unwrap();

    assert!(!created.status.reachable);
    assert!(created.status.error.is_some());
    assert!(created.status.checked_at.is_some());
    assert_eq!(hosts, vec![created]);
}

#[tokio::test]
async fn duplicate_name_is_refused() {
    let (_dir, app) = scratch_app().await;
    create(&app, NAME, UNREACHABLE_URL, MAX_INSTANCES)
        .await
        .unwrap();

    let second = create(&app, NAME, UNREACHABLE_URL, MAX_INSTANCES).await;

    assert!(matches!(
        second,
        Err(Error::Conflict {
            reason: ConflictReason::NameTaken,
            ..
        })
    ));
}

#[tokio::test]
async fn bad_input_is_refused() {
    let (_dir, app) = scratch_app().await;

    let scheme = create(&app, NAME, "tcp://127.0.0.1:2375", MAX_INSTANCES).await;
    let option = create(&app, NAME, "ssh://-oProxyCommand=x@host", MAX_INSTANCES).await;
    let too_many = create(&app, NAME, UNREACHABLE_URL, 65).await;
    let none = create(&app, NAME, UNREACHABLE_URL, 0).await;
    let listed = call(&app, "host.list", json!({})).await.unwrap();

    assert_eq!(invalid_field(scheme), "docker_url");
    assert_eq!(invalid_field(option), "docker_url");
    assert_eq!(invalid_field(too_many), "max_instances");
    assert_eq!(invalid_field(none), "max_instances");
    assert_eq!(listed["hosts"], json!([]));
}

// AC-6: the Host that failed its probe takes nothing; local takes what fits, the rest is refused.
#[tokio::test]
async fn unreachable_host_takes_no_launch() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[DOCKER_SANDBOX, ONE_INSTANCE]))
        .await
        .unwrap();
    create(&app, NAME, UNREACHABLE_URL, MAX_INSTANCES)
        .await
        .unwrap();

    let first = admit(&app, AdmissionMode::Reject).await;
    let second = admit(&app, AdmissionMode::Reject).await;
    let listed = call(&app, "host.list", json!({})).await.unwrap();
    let hosts: Vec<Host> = serde_json::from_value(listed["hosts"].clone()).unwrap();

    assert_eq!(first, Ok(None));
    assert!(matches!(second, Err(Error::CapExceeded { .. })));
    assert!(!hosts[0].status.reachable);
    assert!(hosts[0].status.error.is_some());
}
