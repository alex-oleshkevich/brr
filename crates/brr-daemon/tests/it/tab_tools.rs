#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::sync::Arc;

use brr_core::{
    ArtifactKind, Capability, ClientKind, Error, IdleOverrides, Instance, InstanceId, Tab,
};
use brr_daemon::{App, CallClient, ops};
use brr_store::{artifact, build};
use serde_json::{Value, json};
use tokio::time;

use common::{KillOnDrop, add_profile, chrome_app, ctx, listen, settled};

const BUTTON_PAGE: &str = "data:text/html,<button onclick=\"document.title='x'\">b</button>";
const OTHER_PAGE: &str = "data:text/html,<p>other</p>";
const TITLED_PAGE: &str = "data:text/html,<title>navigated</title><p>p</p>";
const ARTICLE_PAGE: &str = "data:text/html,<html><head><title>T</title></head><body><article><h1>Heading</h1><p>First paragraph of the article, long enough to read as content for the extractor.</p><p>Second paragraph of the article with more words so the extractor keeps it.</p></article></body></html>";
const DIALOG_PAGE: &str = "data:text/html,<button onclick=\"alert(1)\">b</button>";
const REF_MARK: &str = "[ref=";
const DIALOG_BUDGET: std::time::Duration = std::time::Duration::from_secs(10);
const PNG_BASE64_PREFIX: &str = "iVBORw0KGgo";
const PDF_MAGIC: &[u8] = b"%PDF-";
const PROFILE: &str = "p";

// (name, tool, read_only, capability); every row has the REST path /v1/tabs/{tab_id}/<tool>
const ROWS: [(&str, &str, bool, Option<Capability>); 39] = [
    ("tab.go_back", "go_back", false, None),
    ("tab.go_forward", "go_forward", false, None),
    ("tab.reload", "reload", false, None),
    ("tab.click", "click", false, None),
    ("tab.dblclick", "dblclick", false, None),
    ("tab.hover", "hover", false, None),
    ("tab.type", "type", false, None),
    ("tab.fill", "fill", false, None),
    ("tab.press_key", "press_key", false, None),
    ("tab.select_option", "select_option", false, None),
    ("tab.check", "check", false, None),
    ("tab.uncheck", "uncheck", false, None),
    ("tab.drag", "drag", false, None),
    ("tab.upload_file", "upload_file", false, None),
    ("tab.scroll", "scroll", false, None),
    ("tab.wait_for", "wait_for", false, None),
    ("tab.handle_dialog", "handle_dialog", false, None),
    ("tab.evaluate", "evaluate", false, None),
    ("tab.screenshot", "screenshot", true, None),
    ("tab.pdf", "pdf", true, Some(Capability::Pdf)),
    ("tab.mhtml", "mhtml", true, Some(Capability::Mhtml)),
    ("tab.get_html", "get_html", true, None),
    ("tab.get_text", "get_text", true, None),
    ("tab.get_markdown", "get_markdown", true, None),
    ("tab.snapshot", "snapshot", true, None),
    ("tab.get_links", "get_links", true, None),
    ("tab.get_metadata", "get_metadata", true, None),
    ("tab.cookies_get", "cookies_get", true, None),
    ("tab.cookies_set", "cookies_set", false, None),
    ("tab.cookies_clear", "cookies_clear", false, None),
    ("tab.storage_get", "storage_get", true, None),
    ("tab.storage_set", "storage_set", false, None),
    ("tab.storage_clear", "storage_clear", false, None),
    ("tab.console_messages", "console_messages", true, None),
    ("tab.network_log", "network_log", true, None),
    ("tab.har_start", "har_start", false, Some(Capability::Har)),
    ("tab.har_stop", "har_stop", false, Some(Capability::Har)),
    ("tab.emulate", "emulate", false, None),
    (
        "tab.block_requests",
        "block_requests",
        false,
        Some(Capability::RequestInterception),
    ),
];

async fn call(
    app: &Arc<App>,
    client: &CallClient,
    name: &str,
    input: Value,
) -> Result<Value, Error> {
    let mut context = ctx(app);
    context.client = client.clone();

    ops::dispatch(context, ops::find(name).unwrap(), input).await
}

async fn rest_client(app: &Arc<App>, label: &str) -> CallClient {
    let created = app
        .clients
        .create(app, ClientKind::Rest, label.to_string())
        .await
        .unwrap();

    app.clients.resolve(app, &created.id).await.unwrap()
}

// A scratch app whose Build lists `capabilities`, with one ephemeral Profile `p`.
async fn app_with(
    capabilities: Vec<Capability>,
) -> Option<(tempfile::TempDir, Arc<App>, KillOnDrop)> {
    let (dir, app, build_id) = chrome_app(&[]).await?;
    app.store
        .run(move |c| {
            let mut found = build::get(c, &build_id)?;
            found.capabilities = capabilities;

            build::update(c, &found)
        })
        .await
        .unwrap();
    add_profile(&app, build_id, PROFILE, false, IdleOverrides::default()).await;
    let browsers = KillOnDrop::new(&app, Vec::new());

    Some((dir, app, browsers))
}

async fn launch(
    app: &Arc<App>,
    browsers: &KillOnDrop,
    client: &CallClient,
    url: &str,
) -> (InstanceId, Tab) {
    let launched = call(
        app,
        client,
        "instance.launch",
        json!({"profile": PROFILE, "url": url}),
    )
    .await
    .unwrap();
    let instance: Instance = serde_json::from_value(launched["instance"].clone()).unwrap();
    browsers.track(instance.id);

    (
        instance.id,
        serde_json::from_value(launched["tab"].clone()).unwrap(),
    )
}

async fn stop(app: &Arc<App>, client: &CallClient, instance_id: InstanceId) {
    call(
        app,
        client,
        "instance.stop",
        json!({"instance_id": instance_id}),
    )
    .await
    .unwrap();
}

#[test]
fn registry_rows() {
    for (name, tool, read_only, capability) in ROWS {
        let op = ops::find(name).unwrap_or_else(|| panic!("{name} is not registered"));
        let spec = op.spec;

        assert_eq!(
            spec.http.path,
            format!("/v1/tabs/{{tab_id}}/{tool}"),
            "{name}"
        );
        assert_eq!(spec.mcp_tool, Some(tool), "{name}");
        assert_eq!(spec.read_only, read_only, "{name}");
        assert_eq!(spec.capability, capability, "{name}");
        assert_eq!(spec.cli, ["tab", &tool.replace('_', "-")], "{name}");
    }
}

#[tokio::test]
async fn lease_rules_on_tools() {
    let Some((_dir, app, browsers)) = app_with(Vec::new()).await else {
        return;
    };
    let (a, b) = (rest_client(&app, "a").await, rest_client(&app, "b").await);
    let (instance_id, tab) = launch(&app, &browsers, &a, BUTTON_PAGE).await;
    let leases_before = app.leases.list(None, None).await;

    let blocked = call(
        &app,
        &b,
        "tab.click",
        json!({"tab_id": tab.id, "selector": "button"}),
    )
    .await;
    assert_eq!(
        blocked,
        Err(Error::LeaseHeld {
            client_id: a.client_id.clone()
        })
    );
    let shot = call(&app, &b, "tab.screenshot", json!({"tab_id": tab.id}))
        .await
        .unwrap();
    assert!(shot["inline_base64"].is_string());
    call(&app, &b, "tab.snapshot", json!({"tab_id": tab.id}))
        .await
        .unwrap();
    assert_eq!(app.leases.list(None, None).await, leases_before);

    call(
        &app,
        &a,
        "tab.click",
        json!({"tab_id": tab.id, "selector": "button"}),
    )
    .await
    .unwrap();
    settled(async || {
        let got = call(&app, &a, "tab.get", json!({"tab_id": tab.id}))
            .await
            .unwrap();

        got["title"] == "x"
    })
    .await;

    stop(&app, &a, instance_id).await;
}

#[tokio::test]
async fn refs_and_args() {
    let Some((_dir, app, browsers)) = app_with(Vec::new()).await else {
        return;
    };
    let a = rest_client(&app, "a").await;
    let (instance_id, tab) = launch(&app, &browsers, &a, BUTTON_PAGE).await;

    let snapshot = call(&app, &a, "tab.snapshot", json!({"tab_id": tab.id}))
        .await
        .unwrap();
    let text = snapshot["snapshot"].as_str().unwrap();
    let button_ref = text
        .lines()
        .find(|line| line.contains("button"))
        .and_then(|line| line.split(REF_MARK).nth(1))
        .and_then(|rest| rest.split(']').next())
        .unwrap()
        .to_owned();

    call(
        &app,
        &a,
        "tab.click",
        json!({"tab_id": tab.id, "ref": button_ref}),
    )
    .await
    .unwrap();

    call(
        &app,
        &a,
        "tab.navigate",
        json!({"tab_id": tab.id, "url": OTHER_PAGE}),
    )
    .await
    .unwrap();
    let stale = call(
        &app,
        &a,
        "tab.click",
        json!({"tab_id": tab.id, "ref": button_ref}),
    )
    .await;
    assert_eq!(stale, Err(Error::StaleRef));

    let both = call(
        &app,
        &a,
        "tab.click",
        json!({"tab_id": tab.id, "ref": button_ref, "selector": "button"}),
    )
    .await;
    assert!(matches!(
        both,
        Err(Error::InvalidArgument { field, .. }) if field == "ref"
    ));

    stop(&app, &a, instance_id).await;
}

#[tokio::test]
async fn capture_and_artifacts() {
    let capabilities = vec![Capability::Pdf, Capability::Har];
    let Some((_dir, app, browsers)) = app_with(capabilities).await else {
        return;
    };
    let addr = listen(&app).await;
    let a = rest_client(&app, "a").await;
    let (instance_id, tab) = launch(&app, &browsers, &a, BUTTON_PAGE).await;

    let shot = call(
        &app,
        &a,
        "tab.screenshot",
        json!({"tab_id": tab.id, "full_page": true}),
    )
    .await
    .unwrap();
    assert!(
        shot["inline_base64"]
            .as_str()
            .unwrap()
            .starts_with(PNG_BASE64_PREFIX)
    );
    assert!(shot["artifact_id"].is_null());

    let pdf = call(
        &app,
        &a,
        "tab.pdf",
        json!({"tab_id": tab.id, "return": "url"}),
    )
    .await
    .unwrap();
    assert!(pdf["artifact_id"].is_string());
    let url = pdf["url"].as_str().unwrap();
    let path = &url[url.find("/v1/").unwrap()..];
    let served = reqwest::get(format!("http://{addr}{path}"))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert!(served.starts_with(PDF_MAGIC));

    call(&app, &a, "tab.har_start", json!({"tab_id": tab.id}))
        .await
        .unwrap();
    call(
        &app,
        &a,
        "tab.navigate",
        json!({"tab_id": tab.id, "url": OTHER_PAGE}),
    )
    .await
    .unwrap();
    let har = call(&app, &a, "tab.har_stop", json!({"tab_id": tab.id}))
        .await
        .unwrap();
    let har_id = serde_json::from_value(har["artifact_id"].clone()).unwrap();
    let stored = app
        .store
        .run(move |c| artifact::get(c, &har_id))
        .await
        .unwrap();
    assert_eq!(stored.kind, ArtifactKind::Har);

    call(
        &app,
        &a,
        "tab.navigate",
        json!({"tab_id": tab.id, "url": ARTICLE_PAGE}),
    )
    .await
    .unwrap();
    let markdown = call(&app, &a, "tab.get_markdown", json!({"tab_id": tab.id}))
        .await
        .unwrap();
    assert!(markdown["content"].as_str().unwrap().starts_with("# "));

    stop(&app, &a, instance_id).await;
}

#[tokio::test]
async fn capability_gate() {
    let Some((_dir, app, browsers)) = app_with(Vec::new()).await else {
        return;
    };
    let a = rest_client(&app, "a").await;
    let (instance_id, tab) = launch(&app, &browsers, &a, BUTTON_PAGE).await;
    let b = rest_client(&app, "b").await;
    let leases_before = app.leases.list(None, None).await;

    let gated = call(&app, &b, "tab.pdf", json!({"tab_id": tab.id})).await;
    assert_eq!(
        gated,
        Err(Error::EngineUnsupported {
            capability: Capability::Pdf
        })
    );
    let gated = call(&app, &b, "tab.har_start", json!({"tab_id": tab.id})).await;
    assert_eq!(
        gated,
        Err(Error::EngineUnsupported {
            capability: Capability::Har
        })
    );
    assert_eq!(app.leases.list(None, None).await, leases_before);

    stop(&app, &a, instance_id).await;
}

#[tokio::test]
async fn a_dialog_does_not_stall_the_tool_that_opened_it() {
    let Some((_dir, app, browsers)) = app_with(Vec::new()).await else {
        return;
    };
    let a = rest_client(&app, "a").await;
    let (instance_id, tab) = launch(&app, &browsers, &a, DIALOG_PAGE).await;

    let clicked = time::timeout(
        DIALOG_BUDGET,
        call(
            &app,
            &a,
            "tab.click",
            json!({"tab_id": tab.id, "selector": "button"}),
        ),
    )
    .await
    .unwrap();
    clicked.unwrap();

    call(
        &app,
        &a,
        "tab.handle_dialog",
        json!({"tab_id": tab.id, "accept": true}),
    )
    .await
    .unwrap();

    stop(&app, &a, instance_id).await;
}

#[tokio::test]
async fn only_the_lease_holder_snapshot_assigns_refs() {
    let Some((_dir, app, browsers)) = app_with(Vec::new()).await else {
        return;
    };
    let (a, b) = (rest_client(&app, "a").await, rest_client(&app, "b").await);
    let (instance_id, tab) = launch(&app, &browsers, &a, BUTTON_PAGE).await;

    let mine = call(&app, &a, "tab.snapshot", json!({"tab_id": tab.id}))
        .await
        .unwrap();
    let mine = mine["snapshot"].as_str().unwrap();
    assert!(mine.contains(REF_MARK));
    let button_ref = mine
        .lines()
        .find(|line| line.contains("button"))
        .and_then(|line| line.split(REF_MARK).nth(1))
        .and_then(|rest| rest.split(']').next())
        .unwrap()
        .to_owned();

    let theirs = call(&app, &b, "tab.snapshot", json!({"tab_id": tab.id}))
        .await
        .unwrap();
    let theirs = theirs["snapshot"].as_str().unwrap();
    assert!(theirs.contains("button"));
    assert!(!theirs.contains(REF_MARK));

    call(
        &app,
        &a,
        "tab.click",
        json!({"tab_id": tab.id, "ref": button_ref}),
    )
    .await
    .unwrap();

    stop(&app, &a, instance_id).await;
}

// Chromium reports no title a script sets after the load; tab.get asks the page.
#[tokio::test]
async fn tab_get_follows_a_title_a_script_sets() {
    let Some((_dir, app, browsers)) = app_with(Vec::new()).await else {
        return;
    };
    let a = rest_client(&app, "a").await;
    let (instance_id, tab) = launch(&app, &browsers, &a, BUTTON_PAGE).await;

    call(
        &app,
        &a,
        "tab.evaluate",
        json!({"tab_id": tab.id, "expression": "setTimeout(() => { document.title = 'later'; }, 300)"}),
    )
    .await
    .unwrap();

    settled(async || {
        let got = call(&app, &a, "tab.get", json!({"tab_id": tab.id}))
            .await
            .unwrap();

        got["title"] == "later"
    })
    .await;
    stop(&app, &a, instance_id).await;
}

// The Tab's own title can still be the URL when navigation returns; the answer must not be.
#[tokio::test]
async fn navigate_answers_the_loaded_pages_title() {
    let Some((_dir, app, browsers)) = app_with(Vec::new()).await else {
        return;
    };
    let a = rest_client(&app, "a").await;
    let (instance_id, tab) = launch(&app, &browsers, &a, BUTTON_PAGE).await;

    let navigated = call(
        &app,
        &a,
        "tab.navigate",
        json!({"tab_id": tab.id, "url": TITLED_PAGE}),
    )
    .await
    .unwrap();

    assert_eq!(navigated["title"], "navigated");
    stop(&app, &a, instance_id).await;
}
