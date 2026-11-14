#![allow(clippy::unwrap_used)]

use crate::common;

use std::{sync::Arc, time::Duration};

use brr_core::{
    ClientId, ClientKind, ConflictReason, Empty, Error, Http, InstanceId, Method, OperationSpec,
    TabId, Toolset, schema,
};
use brr_daemon::{
    App, CallClient, Ctx,
    ops::{self, OpRegistration},
    services::leases::{expect_take_over, run_watched},
};
use futures::{FutureExt, future::BoxFuture};
use serde_json::{Value, json};
use test_case::test_case;
use tokio::time;

use common::{ctx, listen, post, scratch_app, settled};

const NOT_A_URL: &str = "notaurl";
const SETTLE: Duration = Duration::from_secs(5);
const STAYS_RUNNING: Duration = Duration::from_millis(200);

static PANICKING: OperationSpec = OperationSpec {
    name: "test.panic",
    group: "test",
    toolset: Toolset::System,
    summary: "panics",
    http: Http {
        method: Method::Get,
        path: "/v1/test/panic",
    },
    cli: &["test", "panic"],
    cli_positional: &[],
    mcp_tool: None,
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<Empty>,
};

static PANICKING_OP: OpRegistration = OpRegistration {
    spec: &PANICKING,
    handler: panic_handler,
};

fn panic_handler(_ctx: Ctx, _input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async { panic!("a dependency panicked") }.boxed()
}

async fn call(app: &Arc<App>, name: &str, input: Value) -> Result<Value, Error> {
    ops::dispatch(ctx(app), ops::find(name).unwrap(), input).await
}

fn field_of(result: Result<Value, Error>) -> String {
    match result {
        Err(Error::InvalidArgument { field, .. }) => field,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

fn call_client(kind: ClientKind) -> CallClient {
    CallClient {
        client_id: ClientId::new(),
        kind,
        label: "test".to_string(),
        implicit: true,
    }
}

#[tokio::test]
async fn a_panicking_op_answers_internal() {
    let (_dir, app) = scratch_app().await;

    let result = ops::dispatch(ctx(&app), &PANICKING_OP, json!({})).await;

    assert!(matches!(result, Err(Error::Internal { .. })), "{result:?}");
}

#[test_case("storage.backend_set"; "set")]
#[test_case("storage.backend_test"; "test")]
#[tokio::test]
async fn storage_endpoint_must_be_an_http_url(op: &str) {
    let (_dir, app) = scratch_app().await;

    for endpoint in ["not a url", "ftp://host", "/relative"] {
        let result = call(
            &app,
            op,
            json!({"backend": "s3", "endpoint": endpoint, "bucket": "b",
                   "access_key_id": "k", "secret_access_key": "s"}),
        )
        .await;

        assert_eq!(field_of(result), "endpoint", "{endpoint}");
    }
}

#[tokio::test]
async fn profile_import_rejects_an_empty_file() {
    let (_dir, app) = scratch_app().await;

    let result = call(
        &app,
        "profile.import",
        json!({"file": "", "passphrase": "x"}),
    )
    .await;

    assert_eq!(field_of(result), "file");
}

#[test_case("captcha.settings_get")]
#[test_case("captcha.balance")]
#[test_case("settings.get")]
#[tokio::test]
async fn unknown_keys_are_refused_on_ops_without_input(op: &str) {
    let (_dir, app) = scratch_app().await;

    let result = call(&app, op, json!({"zz": 1})).await;

    assert_eq!(field_of(result), "zz");
}

#[tokio::test]
async fn unknown_key_is_an_mcp_error_result() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    let response = post(
        addr,
        "",
        "tools/call",
        json!({"name": "captcha_settings_get", "arguments": {"zz": 1}}),
    )
    .await;
    let body: Value = response.json().await.unwrap();

    assert_eq!(body["result"]["isError"], true, "{body}");
}

#[test_case(json!({"name": "settings_get", "arguments": 5}); "arguments not an object")]
#[test_case(json!({"name": "settings_get", "arguments": [1]}); "arguments an array")]
#[test_case(json!({"arguments": {}}); "name missing")]
#[test_case(json!({"name": 5}); "name not a string")]
#[tokio::test]
async fn bad_tool_call_is_invalid_params(params: Value) {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    let response = post(addr, "", "tools/call", params).await;
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap();

    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], -32602, "{body}");
    assert_eq!(body["id"], 1, "{body}");
}

#[test_case(Some("application/json"); "json content type")]
#[test_case(None; "no content type")]
#[tokio::test]
async fn malformed_json_is_a_parse_error(content_type: Option<&str>) {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let mut request = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .body("{not json");
    if let Some(content_type) = content_type {
        request = request.header("Content-Type", content_type);
    }

    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap();

    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], -32700, "{body}");
}

#[tokio::test]
async fn page_urls_are_validated_up_front() {
    let (_dir, app) = scratch_app().await;
    let tab_id = TabId::new().to_string();

    for (op, input) in [
        ("tab.navigate", json!({"tab_id": tab_id, "url": NOT_A_URL})),
        (
            "tab.open",
            json!({"instance_id": InstanceId::new().to_string(), "url": NOT_A_URL}),
        ),
        (
            "instance.launch",
            json!({"profile": "none", "url": NOT_A_URL}),
        ),
    ] {
        assert_eq!(field_of(call(&app, op, input).await), "url", "{op}");
    }

    for url in [
        "about:blank",
        "data:text/html,hi",
        "https://example.com/a?b=1",
        "file:///tmp/x.html",
        "chrome://version",
    ] {
        let result = call(&app, "tab.navigate", json!({"tab_id": tab_id, "url": url})).await;

        assert!(
            matches!(result, Err(Error::NotFound { .. })),
            "{url}: {result:?}"
        );
    }
}

#[tokio::test]
async fn credential_username_is_required_and_unique() {
    let (_dir, app) = scratch_app().await;
    let create = |site: &str, username: &str| {
        call(
            &app,
            "credential.create",
            json!({"site": site, "username": username, "password": "p"}),
        )
    };

    assert_eq!(field_of(create("https://a.example", "").await), "username");

    let first = create("https://a.example", "alice").await.unwrap();
    create("https://a.example", "bob").await.unwrap();
    let other_site = create("https://b.example", "alice").await.unwrap();
    let duplicate = create("https://a.example", "alice").await;
    assert!(
        matches!(
            duplicate,
            Err(Error::Conflict {
                reason: ConflictReason::CredentialExists,
                ..
            })
        ),
        "{duplicate:?}"
    );

    let moved = call(
        &app,
        "credential.update",
        json!({"credential_id": other_site["id"], "site": "https://a.example"}),
    )
    .await;
    assert!(matches!(moved, Err(Error::Conflict { .. })), "{moved:?}");
    let renamed = call(
        &app,
        "credential.update",
        json!({"credential_id": first["id"], "username": "bob"}),
    )
    .await;
    assert!(
        matches!(renamed, Err(Error::Conflict { .. })),
        "{renamed:?}"
    );
    let same = call(
        &app,
        "credential.update",
        json!({"credential_id": first["id"], "username": "alice", "notes": "n"}),
    )
    .await;
    assert!(same.is_ok(), "{same:?}");
    let empty = call(
        &app,
        "credential.update",
        json!({"credential_id": first["id"], "username": ""}),
    )
    .await;
    assert_eq!(field_of(empty), "username");
}

#[test_case("limits.set", json!({"max_tabs_per_instance": 0}), "max_tabs_per_instance")]
#[test_case("limits.set", json!({"max_instances_per_engine": {"chromium": 0, "camoufox": 4}}), "max_instances_per_engine")]
#[test_case("limits.set", json!({"max_instances_per_engine": {"chromium": 8, "camoufox": 0}}), "max_instances_per_engine")]
#[test_case("retention.set", json!({"download_max_mb": 0}), "download_max_mb")]
#[test_case("tab.emulate", json!({"tab_id": TabId::new().to_string(), "geolocation": {"latitude": 200, "longitude": 0}}), "geolocation.latitude")]
#[test_case("tab.emulate", json!({"tab_id": TabId::new().to_string(), "geolocation": {"latitude": 0, "longitude": -181}}), "geolocation.longitude")]
#[test_case("tab.emulate", json!({"tab_id": TabId::new().to_string(), "geolocation": {"latitude": 0, "longitude": 0, "accuracy": -1}}), "geolocation.accuracy")]
#[test_case("tab.emulate", json!({"tab_id": TabId::new().to_string(), "geolocation": {"latitude": 0, "longitude": 0, "zz": 1}}), "geolocation.zz")]
#[test_case("tab.cookies_get", json!({"tab_id": TabId::new().to_string(), "urls": ["nope"]}), "urls")]
#[test_case("credential.create", json!({"site": "https://a.example", "username": "u", "password": "p", "selectors": {"zz": "x"}}), "selectors.zz")]
#[test_case("tab.emulate", json!({"tab_id": TabId::new().to_string(), "viewport": {"width": 0, "height": 600}}), "viewport")]
#[test_case("tab.emulate", json!({"tab_id": TabId::new().to_string(), "viewport": {"width": 800, "height": 0}}), "viewport")]
#[tokio::test]
async fn minimums_are_enforced(op: &str, input: Value, field: &str) {
    let (_dir, app) = scratch_app().await;

    assert_eq!(field_of(call(&app, op, input).await), field);
}

#[test_case("profile.create", json!({"name": "x"}), "template")]
#[test_case("tab.emulate", json!({"tab_id": TabId::new().to_string(), "geolocation": {"latitude": 1}}), "geolocation.longitude")]
#[tokio::test]
async fn a_missing_field_is_named(op: &str, input: Value, field: &str) {
    let (_dir, app) = scratch_app().await;

    assert_eq!(field_of(call(&app, op, input).await), field);
}

#[tokio::test]
async fn serde_errors_name_the_offending_field() {
    let (_dir, app) = scratch_app().await;

    let wrong_type = call(&app, "limits.set", json!({"max_instances": "many"})).await;
    let nested = call(
        &app,
        "limits.set",
        json!({"admission": {"queue_timeout_s": "soon"}}),
    )
    .await;

    assert_eq!(field_of(wrong_type), "max_instances");
    assert_eq!(field_of(nested), "admission.queue_timeout_s");
}

#[tokio::test]
async fn stop_waits_for_other_clients_leases_except_for_the_ui() {
    let (_dir, app) = scratch_app().await;
    let (owner, other, ui) = (
        call_client(ClientKind::Mcp),
        call_client(ClientKind::Cli),
        call_client(ClientKind::Ui),
    );
    let instance_id = InstanceId::new();
    app.leases
        .acquire_tab(&app, &owner, &instance_id, &TabId::new())
        .await
        .unwrap();

    let refused = app.leases.guard_stop(&other, &instance_id).await;
    assert_eq!(
        refused,
        Err(Error::LeaseHeld {
            client_id: owner.client_id.clone()
        })
    );
    app.leases.guard_stop(&owner, &instance_id).await.unwrap();
    app.leases.guard_stop(&ui, &instance_id).await.unwrap();
    app.leases
        .guard_stop(&other, &InstanceId::new())
        .await
        .unwrap();
}

enum Taken {
    Release,
    TakeOver,
    ClientEnd,
}

// A call that holds a Tab Lease and then hangs, like a slow page operation.
fn hanging_call(
    app: &Arc<App>,
    client: &CallClient,
    instance_id: InstanceId,
    tab_id: TabId,
    waits_for_person: bool,
) -> tokio::task::JoinHandle<Result<(), Error>> {
    let (app, client) = (Arc::clone(app), client.clone());

    tokio::spawn(run_watched(async move {
        app.leases
            .guard(&app, &client, &instance_id, Some(&tab_id), false)
            .await?;
        if waits_for_person {
            expect_take_over();
        }

        std::future::pending().await
    }))
}

#[test_case(Taken::Release; "released")]
#[test_case(Taken::TakeOver; "taken over")]
#[test_case(Taken::ClientEnd; "client ended")]
#[tokio::test]
async fn a_call_whose_lease_is_taken_away_ends_with_lease_lost(taken: Taken) {
    let (_dir, app) = scratch_app().await;
    let (agent, human) = (call_client(ClientKind::Mcp), call_client(ClientKind::Ui));
    let (instance_id, tab_id) = (InstanceId::new(), TabId::new());
    let running = hanging_call(&app, &agent, instance_id, tab_id, false);
    settled(async || app.leases.tab_lease(&tab_id).await.is_some()).await;
    let lease_id = app.leases.tab_lease(&tab_id).await.unwrap().lease_id;

    match taken {
        Taken::Release => {
            app.leases.release(&app, &agent, &lease_id).await.unwrap();
        }
        Taken::TakeOver => {
            app.leases
                .take_over(&app, &human, &instance_id, &tab_id)
                .await
                .unwrap();
        }
        Taken::ClientEnd => app
            .leases
            .release_client(&app, &agent.client_id, brr_core::ReleaseReason::ClientEnded)
            .await
            .unwrap(),
    }

    let result = time::timeout(SETTLE, running).await.unwrap().unwrap();
    assert_eq!(result, Err(Error::LeaseLost));
}

#[tokio::test]
async fn a_closed_tab_does_not_make_its_call_lose_the_lease() {
    let (_dir, app) = scratch_app().await;
    let agent = call_client(ClientKind::Mcp);
    let (instance_id, tab_id) = (InstanceId::new(), TabId::new());
    let running = hanging_call(&app, &agent, instance_id, tab_id, false);
    settled(async || app.leases.tab_lease(&tab_id).await.is_some()).await;

    app.leases
        .release_tab(&app, &tab_id, brr_core::ReleaseReason::TabClosed)
        .await
        .unwrap();

    assert!(
        time::timeout(STAYS_RUNNING, running).await.is_err(),
        "the call must keep running"
    );
}

#[tokio::test]
async fn a_call_waiting_for_a_person_survives_the_take_over() {
    let (_dir, app) = scratch_app().await;
    let (agent, human) = (call_client(ClientKind::Mcp), call_client(ClientKind::Ui));
    let (instance_id, tab_id) = (InstanceId::new(), TabId::new());
    let running = hanging_call(&app, &agent, instance_id, tab_id, true);
    settled(async || app.leases.tab_lease(&tab_id).await.is_some()).await;

    app.leases
        .take_over(&app, &human, &instance_id, &tab_id)
        .await
        .unwrap();

    assert!(
        time::timeout(STAYS_RUNNING, running).await.is_err(),
        "the call must keep running"
    );
}

#[test_case("tab.open", json!({"instance_id": "{id}"}))]
#[test_case("lease.acquire", json!({"instance_id": "{id}", "exclusive": true}))]
#[test_case("lease.acquire", json!({"instance_id": "{id}", "tab_id": "{tab}"}))]
#[tokio::test]
async fn an_ended_instance_is_an_invalid_transition(op: &str, input: Value) {
    let (_dir, app) = scratch_app().await;
    let instance = common::instance_row(brr_core::InstanceState::Stopped, None);
    let tab = common::tab_row(instance.id, brr_core::TabState::Closed);
    let (instance_id, tab_id) = (instance.id, tab.id);
    app.store
        .run(move |c| {
            brr_store::instance::insert(c, &instance)?;
            brr_store::tab::insert(c, &tab)
        })
        .await
        .unwrap();
    let input = input
        .to_string()
        .replace("{id}", &instance_id.to_string())
        .replace("{tab}", &tab_id.to_string());

    let result = call(&app, op, serde_json::from_str(&input).unwrap()).await;

    assert!(
        matches!(&result, Err(Error::InvalidTransition { from, .. }) if from == "stopped"),
        "{result:?}"
    );
}

async fn seed_instance(app: &Arc<App>, state: brr_core::InstanceState) -> InstanceId {
    let instance = common::instance_row(state, None);
    let instance_id = instance.id;
    app.store
        .run(move |c| brr_store::instance::insert(c, &instance))
        .await
        .unwrap();

    instance_id
}

#[tokio::test]
async fn stop_checks_the_state_before_the_leases() {
    let (_dir, app) = scratch_app().await;
    let instance_id = seed_instance(&app, brr_core::InstanceState::Attached).await;
    let owner = call_client(ClientKind::Mcp);
    app.leases
        .acquire_tab(&app, &owner, &instance_id, &TabId::new())
        .await
        .unwrap();

    let result = call(
        &app,
        "instance.stop",
        json!({"instance_id": instance_id.to_string()}),
    )
    .await;

    assert!(
        matches!(result, Err(Error::InvalidTransition { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn forced_stop_ends_the_holders_calls() {
    let (_dir, app) = scratch_app().await;
    let instance_id = seed_instance(&app, brr_core::InstanceState::Running).await;
    let (owner, tab_id) = (call_client(ClientKind::Mcp), TabId::new());
    let running = hanging_call(&app, &owner, instance_id, tab_id, false);
    settled(async || app.leases.tab_lease(&tab_id).await.is_some()).await;
    let input = json!({"instance_id": instance_id.to_string()});

    let plain = call(&app, "instance.stop", input.clone()).await;
    assert!(matches!(plain, Err(Error::LeaseHeld { .. })), "{plain:?}");

    // No owner task runs the seeded Instance, so the stop itself fails; the leases are gone by then.
    let _ = call(
        &app,
        "instance.stop",
        json!({"instance_id": instance_id.to_string(), "force": true}),
    )
    .await;

    let result = time::timeout(SETTLE, running).await.unwrap().unwrap();
    assert_eq!(result, Err(Error::LeaseLost));
    assert!(app.leases.list(Some(&instance_id), None).await.is_empty());
}
