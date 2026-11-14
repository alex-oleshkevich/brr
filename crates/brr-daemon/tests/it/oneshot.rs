#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use std::collections::BTreeMap;

use brr_core::{
    BuildId, Capability, ClientKind, Error, InstanceId, InstanceState, ProfileKind, TabState,
};
use brr_daemon::{App, CallClient, ops};
use brr_store::{build, profile};
use serde_json::{Value, json};
use tokio::time;

use common::{KillOnDrop, chrome_app, ctx, listen, row, wait_for};

const PAGE: &str = "data:text/html,<body style=\"background:#fff\">hi</body>";
const LINKS_PAGE: &str =
    "data:text/html,<a href=\"https://example.com/a\">A</a><a href=\"https://example.com/b\">B</a>";
const ARTICLE_PAGE: &str = "data:text/html,<html><head><title>T</title></head><body><article><h1>Heading</h1><p>First paragraph of the article, long enough to read as content for the extractor.</p><p>Second paragraph of the article with more words so the extractor keeps it.</p></article></body></html>";
const ONESHOT_PROFILE: &str = "oneshot";
const PNG_MAGIC: &[u8] = b"\x89PNG";
// base64 of "%PDF-"
const PDF_BASE64_PREFIX: &str = "JVBERi0";
const HOLD_MS: u64 = 1500;
const SETTLE: Duration = Duration::from_secs(5);

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

async fn grant(app: &Arc<App>, build_id: BuildId, capabilities: Vec<Capability>) {
    app.store
        .run(move |c| {
            let mut found = build::get(c, &build_id)?;
            found.capabilities = capabilities;

            build::update(c, &found)
        })
        .await
        .unwrap();
}

fn oneshot_instances(app: &Arc<App>) -> Vec<brr_daemon::services::instances::InstanceView> {
    app.instances
        .views()
        .into_iter()
        .filter(|view| view.instance.state != InstanceState::Stopped)
        .collect()
}

fn open_tabs(view: &brr_daemon::services::instances::InstanceView) -> usize {
    view.tabs
        .iter()
        .filter(|tab| tab.state != TabState::Closed)
        .count()
}

async fn get(addr: SocketAddr, path: &str, query: &[(&str, &str)]) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("http://{addr}{path}"))
        .query(query)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn raw_endpoints() {
    let Some((_dir, app, _build)) = chrome_app(&[]).await else {
        return;
    };
    let browsers = KillOnDrop::new(&app, Vec::new());
    let addr = listen(&app).await;

    let shot = get(
        addr,
        "/v1/screenshot",
        &[("url", PAGE), ("screen", "800x600"), ("theme", "dark")],
    )
    .await;
    assert_eq!(shot.status(), 200);
    assert_eq!(shot.headers()["content-type"], "image/png");
    let png = shot.bytes().await.unwrap();
    assert!(png.starts_with(PNG_MAGIC));
    let (width, height) = (
        u32::from_be_bytes(png[16..20].try_into().unwrap()),
        u32::from_be_bytes(png[20..24].try_into().unwrap()),
    );
    assert_eq!((width, height), (800, 600));

    let found = app
        .store
        .run(|c| profile::find(c, ONESHOT_PROFILE))
        .await
        .unwrap();
    assert_eq!(found.kind, ProfileKind::Ephemeral);

    let links = get(addr, "/v1/links", &[("url", LINKS_PAGE)]).await;
    assert_eq!(links.headers()["content-type"], "application/json");
    let links: Vec<Value> = links.json().await.unwrap();
    assert_eq!(links.len(), 2);
    assert_eq!(links[0]["href"], "https://example.com/a");
    assert_eq!(links[0]["text"], "A");

    let markdown = reqwest::Client::new()
        .post(format!("http://{addr}/v1/markdown"))
        .json(&json!({"url": ARTICLE_PAGE}))
        .send()
        .await
        .unwrap();
    assert!(
        markdown.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/markdown")
    );
    assert!(markdown.text().await.unwrap().starts_with("# "));

    app.shutdown().await;
    drop(browsers);
}

#[tokio::test]
async fn return_modes() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    grant(&app, build_id, vec![Capability::Pdf]).await;
    let browsers = KillOnDrop::new(&app, Vec::new());
    let addr = listen(&app).await;

    let both = reqwest::Client::new()
        .post(format!("http://{addr}/v1/pdf"))
        .json(&json!({"url": PAGE, "return": "both"}))
        .send()
        .await
        .unwrap();
    assert_eq!(both.status(), 200);
    let body: Value = both.json().await.unwrap();
    assert!(body["artifact_id"].is_string());
    assert!(body["url"].is_string());
    assert!(body["expires_at"].is_number());
    assert!(body["size"].is_number());
    assert_eq!(body["mime"], "application/pdf");
    assert!(
        body["inline_base64"]
            .as_str()
            .unwrap()
            .starts_with(PDF_BASE64_PREFIX)
    );

    let bogus = get(addr, "/v1/screenshot", &[("url", PAGE), ("bogus", "1")]).await;
    assert_eq!(bogus.status(), 400);
    let body: Value = bogus.json().await.unwrap();
    assert_eq!(body["code"], "InvalidArgument");

    app.shutdown().await;
    drop(browsers);
}

#[tokio::test]
async fn pool_and_linger() {
    let Some((_dir, app, _build)) = chrome_app(&[("BRR_ONESHOT__POOL_SIZE", "1")]).await else {
        return;
    };
    let browsers = KillOnDrop::new(&app, Vec::new());

    wait_for("the pooled instance", async || {
        oneshot_instances(&app)
            .iter()
            .any(|view| view.instance.state == InstanceState::Running)
    })
    .await;
    let pooled = oneshot_instances(&app)[0].instance.id;
    browsers.track(pooled);
    let baseline = open_tabs(&oneshot_instances(&app)[0]);

    let client = rest_client(&app, "a").await;
    let slow = json!({"url": PAGE, "delay": HOLD_MS});
    let requests = tokio::spawn({
        let (app, client, slow) = (Arc::clone(&app), client.clone(), slow.clone());
        async move {
            let first = call(&app, &client, "oneshot.content", slow.clone());
            let second = call(&app, &client, "oneshot.content", slow);

            tokio::join!(first, second)
        }
    });
    wait_for("two contexts in one instance", async || {
        let views = oneshot_instances(&app);

        views.len() == 1 && open_tabs(&views[0]) == baseline + 2
    })
    .await;
    let (first, second) = requests.await.unwrap();
    first.unwrap();
    second.unwrap();
    assert_eq!(oneshot_instances(&app).len(), 1);
    app.shutdown().await;
    drop(browsers);

    let Some((_dir, app, _build)) = chrome_app(&[("BRR_ONESHOT__LINGER_S", "1")]).await else {
        return;
    };
    let browsers = KillOnDrop::new(&app, Vec::new());
    let client = rest_client(&app, "a").await;
    call(&app, &client, "oneshot.content", json!({"url": PAGE}))
        .await
        .unwrap();
    let lingering = oneshot_instances(&app)[0].instance.id;
    browsers.track(lingering);

    time::timeout(SETTLE, async {
        while !row(&app, lingering).await.state.is_final() {
            time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancel_and_exclusive() {
    let Some((_dir, app, _build)) = chrome_app(&[]).await else {
        return;
    };
    let browsers = KillOnDrop::new(&app, Vec::new());
    let client = rest_client(&app, "a").await;

    let request = tokio::spawn({
        let (app, client) = (Arc::clone(&app), client.clone());
        async move {
            call(
                &app,
                &client,
                "oneshot.content",
                json!({"url": PAGE, "delay": 20_000}),
            )
            .await
        }
    });
    wait_for("the request's context", async || {
        oneshot_instances(&app)
            .first()
            .is_some_and(|view| open_tabs(view) == 2)
    })
    .await;
    let instance_id = oneshot_instances(&app)[0].instance.id;
    browsers.track(instance_id);

    request.abort();
    let _ = request.await;
    time::timeout(SETTLE, async {
        while open_tabs(&oneshot_instances(&app)[0]) != 1 {
            time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();

    let created = call(
        &app,
        &client,
        "profile.create",
        json!({"name": "keeper", "template": "chromium-headless", "kind": "persistent"}),
    )
    .await
    .unwrap();
    assert_eq!(created["name"], "keeper");
    let launched = call(
        &app,
        &client,
        "instance.launch",
        json!({"profile": "keeper", "exclusive": true}),
    )
    .await
    .unwrap();
    let kept: InstanceId = serde_json::from_value(launched["instance"]["id"].clone()).unwrap();
    browsers.track(kept);

    let other = rest_client(&app, "b").await;
    let blocked = call(
        &app,
        &other,
        "oneshot.content",
        json!({"url": PAGE, "profile": "keeper"}),
    )
    .await;
    assert_eq!(
        blocked,
        Err(Error::LeaseHeld {
            client_id: client.client_id.clone()
        })
    );

    app.shutdown().await;
}

#[tokio::test]
async fn capability_is_checked_before_anything_launches() {
    let Some((_dir, app, _build)) = chrome_app(&[]).await else {
        return;
    };
    let client = rest_client(&app, "a").await;

    let refused = call(&app, &client, "oneshot.pdf", json!({"url": PAGE})).await;

    assert_eq!(
        refused,
        Err(Error::EngineUnsupported {
            capability: Capability::Pdf
        })
    );
    assert!(app.instances.views().is_empty());
}

#[tokio::test]
async fn cookies_are_refused_for_a_persistent_profile() {
    let Some((_dir, app, _build)) = chrome_app(&[]).await else {
        return;
    };
    let client = rest_client(&app, "a").await;
    call(
        &app,
        &client,
        "profile.create",
        json!({"name": "keeper", "template": "chromium-headless", "kind": "persistent"}),
    )
    .await
    .unwrap();

    let refused = call(
        &app,
        &client,
        "oneshot.content",
        json!({"url": PAGE, "profile": "keeper", "cookies": []}),
    )
    .await;

    assert!(matches!(
        refused,
        Err(Error::InvalidArgument { field, .. }) if field == "cookies"
    ));
    assert!(app.instances.views().is_empty());
}

#[tokio::test]
async fn the_oneshot_profile_is_built_in_and_comes_back_when_missing() {
    let Some((_dir, app, _build)) = chrome_app(&[]).await else {
        return;
    };
    let browsers = KillOnDrop::new(&app, Vec::new());
    let client = rest_client(&app, "a").await;

    let shown = call(
        &app,
        &client,
        "profile.get",
        json!({"profile": ONESHOT_PROFILE}),
    )
    .await
    .unwrap();
    assert_eq!(shown["builtin"], true);
    let refused = call(
        &app,
        &client,
        "profile.delete",
        json!({"profile": ONESHOT_PROFILE}),
    )
    .await;
    assert!(matches!(
        refused,
        Err(Error::InvalidArgument { field, .. }) if field == "profile"
    ));

    // A name nobody made yet: the first one-shot makes the profile.
    app.set_settings(BTreeMap::from([(
        "oneshot.profile".to_string(),
        json!("fresh"),
    )]))
    .await
    .unwrap();
    call(&app, &client, "oneshot.content", json!({"url": PAGE}))
        .await
        .unwrap();
    for view in app.instances.views() {
        browsers.track(view.instance.id);
    }
    let made = call(&app, &client, "profile.get", json!({"profile": "fresh"}))
        .await
        .unwrap();
    assert_eq!(made["builtin"], true);

    app.shutdown().await;
}

#[tokio::test]
async fn pool_members_stay_pinned_across_a_profile_update() {
    let Some((_dir, app, _build)) = chrome_app(&[("BRR_ONESHOT__POOL_SIZE", "1")]).await else {
        return;
    };
    let browsers = KillOnDrop::new(&app, Vec::new());
    let client = rest_client(&app, "a").await;

    wait_for("the pinned pool member", async || {
        oneshot_instances(&app)
            .iter()
            .any(|view| view.instance.state == InstanceState::Running && view.pinned)
    })
    .await;
    browsers.track(oneshot_instances(&app)[0].instance.id);

    let instance = &oneshot_instances(&app)[0];
    let (instance_id, tab_id) = (instance.instance.id, instance.tabs[0].id);

    // A Profile update recomputes the pin from the Profile; the pool membership stays.
    for pinned in [true, false] {
        call(
            &app,
            &client,
            "profile.update",
            json!({"profile": ONESHOT_PROFILE, "pinned": pinned}),
        )
        .await
        .unwrap();
        // A command round trip returns only after the owner handled the update; the second one
        // also waits for the view the first one published.
        for _ in 0..2 {
            app.instances
                .set_tab_pinned(&instance_id, &tab_id, false)
                .await
                .unwrap();
        }

        assert!(oneshot_instances(&app)[0].pinned);
    }
    app.shutdown().await;
}

const NO_REPLACEMENT_WINDOW: Duration = Duration::from_millis(1500);

#[tokio::test]
async fn shutdown_does_not_refill_the_pool() {
    let Some((_dir, app, _build)) = chrome_app(&[("BRR_ONESHOT__POOL_SIZE", "1")]).await else {
        return;
    };
    let browsers = KillOnDrop::new(&app, Vec::new());
    wait_for("the pooled instance", async || {
        oneshot_instances(&app)
            .iter()
            .any(|view| view.instance.state == InstanceState::Running)
    })
    .await;
    browsers.track(oneshot_instances(&app)[0].instance.id);

    app.shutdown().await;

    let started = time::Instant::now();
    while started.elapsed() < NO_REPLACEMENT_WINDOW {
        assert!(oneshot_instances(&app).is_empty(), "the pool was refilled");
        time::sleep(Duration::from_millis(50)).await;
    }
}
