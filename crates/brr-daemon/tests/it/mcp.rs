#![allow(clippy::unwrap_used)]

use crate::common;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use brr_core::ClientEndReason;
use brr_core::{ClientId, Error, Toolset};
use brr_daemon::{
    App,
    mcp::{Ensure, Forwarder},
    ops,
    remote::Remote,
    serve,
};
use serde_json::{Value, json};
use tokio::{net::UnixStream, sync::oneshot, time};

use common::{boot, ctx, listen, post, rpc, scratch_app, settled, tool_names};

const SYSTEM_TOOLSET: &str = "?toolsets=system";
const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const EPHEMERAL_TCP: (&str, &str) = ("BRR_SERVER__BIND", "127.0.0.1:0");
const ERROR_PREFIX: &str = "NotFound: ";
const BODY_LINE: usize = 1;

async fn call(addr: SocketAddr, tool: &str, arguments: Value) -> Value {
    let body = rpc(
        addr,
        "",
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
    )
    .await;

    body["result"].clone()
}

#[tokio::test]
async fn tools_list_and_toolsets() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    let mut listed = tool_names(addr, "").await;
    let mut expected: Vec<String> = ops::all()
        .iter()
        .filter_map(|op| op.spec.mcp_tool.map(str::to_string))
        .collect();
    listed.sort();
    expected.sort();
    assert_eq!(listed, expected);

    let mut system = tool_names(addr, SYSTEM_TOOLSET).await;
    let mut expected: Vec<String> = ops::all()
        .iter()
        .filter(|op| op.spec.toolset == Toolset::System)
        .filter_map(|op| op.spec.mcp_tool.map(str::to_string))
        .collect();
    system.sort();
    expected.sort();
    assert!(!system.is_empty());
    assert!(system.len() < listed.len());
    assert_eq!(system, expected);

    let bad = post(addr, "?toolsets=nope", "tools/list", json!({})).await;
    let body: Value = bad.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("toolset"),
        "{body}"
    );
}

fn tools_of(toolset: Toolset) -> Vec<String> {
    let mut tools: Vec<String> = ops::all()
        .iter()
        .filter(|op| op.spec.toolset == toolset)
        .filter_map(|op| op.spec.mcp_tool.map(str::to_string))
        .collect();
    tools.sort();

    tools
}

#[tokio::test]
async fn default_toolsets_setting_applies_without_a_query() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    ops::dispatch(
        ctx(&app),
        ops::find("settings.set").unwrap(),
        json!({"mcp.default_toolsets": ["system"]}),
    )
    .await
    .unwrap();
    let mut defaulted = tool_names(addr, "").await;
    defaulted.sort();
    let mut asked = tool_names(addr, "?toolsets=browser").await;
    asked.sort();
    let hidden = post(
        addr,
        "",
        "tools/call",
        json!({"name": "client_create", "arguments": {}}),
    )
    .await;
    let hidden: Value = hidden.json().await.unwrap();

    assert_eq!(defaulted, tools_of(Toolset::System));
    assert_eq!(asked, tools_of(Toolset::Browser));
    assert!(
        hidden["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown tool"),
        "{hidden}"
    );
}

#[tokio::test]
async fn call_success_and_error() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    let created = call(addr, "client_create", json!({})).await;
    let client = &created["structuredContent"];
    assert_eq!(client["kind"], "mcp");
    let client_id = client["id"].as_str().unwrap();

    let settings = call(addr, "settings_get", json!({ "client_id": client_id })).await;
    assert!(
        settings["structuredContent"]["settings"].is_object(),
        "{settings}"
    );
    assert_eq!(settings["isError"], false);

    let failed = call(addr, "profile_get", json!({ "profile": "nope" })).await;
    assert_eq!(failed["isError"], true);
    let text = failed["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with(ERROR_PREFIX), "{text}");
    let body: Value = serde_json::from_str(text.lines().nth(BODY_LINE).unwrap()).unwrap();
    assert_eq!(body["code"], "NotFound");
    assert!(failed.get("structuredContent").is_none());

    let expired = call(
        addr,
        "settings_get",
        json!({ "client_id": ClientId::new().as_str() }),
    )
    .await;
    assert_eq!(expired["isError"], true);
    assert!(
        expired["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("client_create")
    );
}

#[tokio::test]
async fn declared_client_id_is_the_op_argument() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    let created = call(addr, "client_create", json!({})).await;
    let client_id = created["structuredContent"]["id"].as_str().unwrap();

    let leases = call(addr, "lease_list", json!({ "client_id": client_id })).await;
    assert_eq!(leases["isError"], false, "{leases}");
    assert!(
        leases["structuredContent"]["leases"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let ended = call(addr, "client_end", json!({ "client_id": client_id })).await;
    assert_eq!(ended["isError"], false, "{ended}");
    assert_eq!(ended["structuredContent"]["id"], client_id);
    assert!(ended["structuredContent"]["ended_at"].is_number());
}

#[tokio::test]
async fn no_host_check() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    let response = reqwest::Client::new()
        .post(format!("http://{addr}/mcp"))
        .header("Host", "example.com")
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }))
        .send()
        .await
        .unwrap();

    assert_ne!(response.status(), 403);
}

#[tokio::test]
async fn remote_client() {
    let dir = tempfile::tempdir().unwrap();
    let app: Arc<App> = App::start(boot(dir.path(), &[EPHEMERAL_TCP]))
        .await
        .unwrap();
    let socket = app.paths.socket(&app.settings());
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(serve(Arc::clone(&app), async {
        let _ = stopped.await;
    }));
    settled(async || UnixStream::connect(&socket).await.is_ok()).await;
    let remote = Remote::new(&socket).unwrap();

    assert!(remote.is_up().await);
    let settings = remote.call("settings.get", &json!({}), None).await.unwrap();
    assert!(settings["settings"].is_object());

    let missing = remote
        .call("profile.get", &json!({ "profile": "x" }), None)
        .await;
    assert!(
        matches!(missing, Err(Error::NotFound { .. })),
        "{missing:?}"
    );

    stop.send(()).unwrap();
    time::timeout(SETTLE_TIMEOUT, server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert!(!remote.is_up().await);
    let down = remote.call("settings.get", &json!({}), None).await;
    assert!(
        matches!(&down, Err(Error::DependencyMissing { name, .. }) if name == "daemon"),
        "{down:?}"
    );
}

#[tokio::test]
async fn forwarder_registers_a_new_client_when_its_own_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let app: Arc<App> = App::start(boot(dir.path(), &[EPHEMERAL_TCP]))
        .await
        .unwrap();
    let socket = app.paths.socket(&app.settings());
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(serve(Arc::clone(&app), async {
        let _ = stopped.await;
    }));
    settled(async || UnixStream::connect(&socket).await.is_ok()).await;
    let ensure: Ensure = Box::new(|| Box::pin(async { Ok(()) }));
    let forwarder = Forwarder::new(Remote::new(&socket).unwrap(), "test", ensure);

    let first = forwarder.client_id().await.unwrap();
    forwarder.call("settings.get", &json!({})).await.unwrap();

    // What a daemon restart does to every Client it had.
    app.clients
        .end(&app, &first, ClientEndReason::DaemonRestart)
        .await
        .unwrap();
    let answered = forwarder.call("settings.get", &json!({})).await.unwrap();

    assert!(answered["settings"].is_object());
    assert_ne!(forwarder.client_id().await.unwrap(), first);

    stop.send(()).unwrap();
    time::timeout(SETTLE_TIMEOUT, server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
