use std::{net::SocketAddr, sync::Arc, time::Duration};

use brr_core::{
    Build, BuildId, BuildSource, BuildState, ClientKind, Engine, IdleOverrides, InstanceId,
    InstanceState,
};
use brr_daemon::{App, CallClient, ops};
use brr_store::{build, client};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{net::TcpStream, time};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{self, Message},
};

use crate::common::{KillOnDrop, add_profile, chrome_app, ctx, events_of, listen, row, wait_for};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const READ_TIMEOUT: Duration = Duration::from_secs(15);
const LOCKED: u16 = 423;
const BAD_REQUEST: u16 = 400;
const STATE_CHANGED: &str = "instance.state_changed";

async fn connect(addr: SocketAddr, query: &str) -> Socket {
    connect_async(format!("ws://{addr}/cdp{query}"))
        .await
        .unwrap()
        .0
}

async fn refused(addr: SocketAddr, query: &str) -> u16 {
    match connect_async(format!("ws://{addr}/cdp{query}")).await {
        Err(tungstenite::Error::Http(response)) => response.status().as_u16(),
        other => panic!("expected an HTTP refusal, got {:?}", other.map(|_| ())),
    }
}

async fn next_frame(socket: &mut Socket) -> Option<Value> {
    loop {
        match time::timeout(READ_TIMEOUT, socket.next()).await.unwrap() {
            Some(Ok(Message::Text(text))) => return Some(serde_json::from_str(&text).unwrap()),
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => {}
        }
    }
}

// The answer to `id`, and the events that came before it.
async fn request(socket: &mut Socket, id: u64, method: &str, params: Value) -> (Value, Vec<Value>) {
    let frame = json!({"id": id, "method": method, "params": params});
    socket.send(Message::text(frame.to_string())).await.unwrap();

    let mut events = Vec::new();
    loop {
        let frame = next_frame(socket).await.unwrap();
        if frame["id"] == json!(id) {
            return (frame, events);
        }
        events.push(frame);
    }
}

fn targets(answer: &Value) -> Vec<Value> {
    answer["result"]["targetInfos"].as_array().unwrap().clone()
}

async fn only_instance(app: &Arc<App>) -> InstanceId {
    let views = app.instances.views();
    assert_eq!(views.len(), 1);

    views[0].instance.id
}

#[tokio::test]
async fn connect_and_list() {
    let Some((_dir, app, _build)) = chrome_app(&[]).await else {
        return;
    };
    let browsers = KillOnDrop::new(&app, Vec::new());
    let addr = listen(&app).await;
    let mut socket = connect(addr, "").await;

    let (_, created) = request(
        &mut socket,
        1,
        "Target.setDiscoverTargets",
        json!({"discover": true}),
    )
    .await;
    let (answer, _) = request(&mut socket, 2, "Target.getTargets", json!({})).await;

    assert!(
        created
            .iter()
            .any(|event| event["method"] == "Target.targetCreated")
    );
    assert!(
        targets(&answer)
            .iter()
            .any(|target| target["type"] == "page")
    );
    let instance_id = only_instance(&app).await;
    browsers.track(instance_id);
    let leases = app.leases.list(Some(&instance_id), None).await;
    let held = leases.iter().find(|lease| lease.exclusive).unwrap();
    assert_eq!(held.client_kind, ClientKind::Connect);

    socket.close(None).await.unwrap();
    app.shutdown().await;
}

#[tokio::test]
async fn clients_isolated() {
    let Some((_dir, app, _build)) = chrome_app(&[]).await else {
        return;
    };
    let browsers = KillOnDrop::new(&app, Vec::new());
    let addr = listen(&app).await;
    let mut one = connect(addr, "").await;
    let mut two = connect(addr, "").await;
    for view in app.instances.views() {
        browsers.track(view.instance.id);
    }
    assert_eq!(app.instances.views().len(), 2);

    let (created, _) = request(
        &mut one,
        1,
        "Target.createTarget",
        json!({"url": "data:text/html,one"}),
    )
    .await;
    assert!(created["result"]["targetId"].is_string());

    let (mine, _) = request(&mut one, 2, "Target.getTargets", json!({})).await;
    let (theirs, _) = request(&mut two, 1, "Target.getTargets", json!({})).await;
    let created_id = created["result"]["targetId"].clone();
    let has_one = |answer: &Value| {
        targets(answer)
            .iter()
            .any(|target| target["targetId"] == created_id || target["url"] == "data:text/html,one")
    };
    assert!(has_one(&mine));
    assert!(!has_one(&theirs));

    app.shutdown().await;
}

// The Instance is launched outside the connection, so only Browser.close can stop it: the
// connection's own cleanup leaves an Instance it did not launch alone.
#[tokio::test]
async fn browser_close_is_stop() {
    let Some((_dir, app, _build)) = chrome_app(&[]).await else {
        return;
    };
    let browsers = KillOnDrop::new(&app, Vec::new());
    let addr = listen(&app).await;

    let created = app
        .clients
        .create(&app, ClientKind::Rest, "x".to_string())
        .await
        .unwrap();
    let owner = app.clients.resolve(&app, &created.id).await.unwrap();
    call(
        &app,
        &owner,
        "profile.create",
        json!({"name": "keeper", "template": "chromium-headless", "kind": "persistent"}),
    )
    .await;
    let launched = call(
        &app,
        &owner,
        "instance.launch",
        json!({"profile": "keeper"}),
    )
    .await;
    let instance_id: InstanceId =
        serde_json::from_value(launched["instance"]["id"].clone()).unwrap();
    browsers.track(instance_id);
    // The launch's Tab Lease would refuse the connection.
    app.clients
        .end(&app, &created.id, brr_core::ClientEndReason::Disconnected)
        .await
        .unwrap();
    let downloads = app.paths.downloads_dir().join(instance_id.to_string());
    let listing = || {
        let mut names: Vec<_> = std::fs::read_dir(&downloads)
            .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
            .unwrap_or_default();
        names.sort();
        names
    };
    let before = listing();

    let mut socket = connect(addr, "?profile=keeper").await;
    assert_eq!(only_instance(&app).await, instance_id);

    let (download, _) = request(
        &mut socket,
        1,
        "Browser.setDownloadBehavior",
        json!({"behavior": "allow", "downloadPath": "/tmp/elsewhere"}),
    )
    .await;
    assert_eq!(download["result"], json!({}));
    assert_eq!(listing(), before);
    assert_eq!(row(&app, instance_id).await.state, InstanceState::Running);

    let (closed, _) = request(&mut socket, 2, "Browser.close", json!({})).await;
    assert_eq!(closed["result"], json!({}));
    assert!(next_frame(&mut socket).await.is_none());

    wait_for("the instance to stop", async || {
        row(&app, instance_id).await.state == InstanceState::Stopped
    })
    .await;
    let changes = events_of(&app, STATE_CHANGED, instance_id).await;
    let step = |to: &str| changes.iter().find(|change| change["to"] == to).cloned();
    assert_eq!(step("stopping").unwrap()["trigger"], "stop");
    assert!(step("crashed").is_none());
    assert!(step("stopped").is_some());
    assert_eq!(listing(), before);
}

async fn call(app: &Arc<App>, client: &CallClient, name: &str, input: Value) -> Value {
    let mut context = ctx(app);
    context.client = client.clone();

    ops::dispatch(context, ops::find(name).unwrap(), input)
        .await
        .unwrap()
}

#[tokio::test]
async fn refusals_and_cleanup() {
    let Some((_dir, app, _build)) = chrome_app(&[("BRR_LEASE__EXCLUSIVE_WAIT_S", "1")]).await
    else {
        return;
    };
    let browsers = KillOnDrop::new(&app, Vec::new());
    let addr = listen(&app).await;

    let created = app
        .clients
        .create(&app, ClientKind::Rest, "x".to_string())
        .await
        .unwrap();
    let holder = app.clients.resolve(&app, &created.id).await.unwrap();
    call(
        &app,
        &holder,
        "profile.create",
        json!({"name": "keeper", "template": "chromium-headless", "kind": "persistent"}),
    )
    .await;
    let launched = call(
        &app,
        &holder,
        "instance.launch",
        json!({"profile": "keeper", "exclusive": true}),
    )
    .await;
    browsers.track(serde_json::from_value(launched["instance"]["id"].clone()).unwrap());
    assert_eq!(refused(addr, "?profile=keeper").await, LOCKED);

    let camoufox = Build {
        id: BuildId::new(),
        source: BuildSource::Camoufox,
        engine: Engine::Camoufox,
        name: "Camoufox".to_owned(),
        version: "1.0.0".to_owned(),
        channel: None,
        path: "/nonexistent".into(),
        state: BuildState::Registered,
        sha256: None,
        size_bytes: 0,
        capabilities: Vec::new(),
        installed_at: None,
        last_referenced_at: None,
    };
    let camoufox_id = camoufox.id;
    app.store
        .run(move |c| build::insert(c, &camoufox))
        .await
        .unwrap();
    add_profile(&app, camoufox_id, "fox", false, IdleOverrides::default()).await;
    assert_eq!(refused(addr, "?profile=fox").await, BAD_REQUEST);
    assert_eq!(refused(addr, "?bogus=1").await, BAD_REQUEST);

    let before: Vec<InstanceId> = app
        .instances
        .views()
        .iter()
        .map(|v| v.instance.id)
        .collect();
    let mut socket = connect(addr, "").await;
    let launched_here = app
        .instances
        .views()
        .into_iter()
        .map(|view| view.instance.id)
        .find(|id| !before.contains(id))
        .unwrap();
    browsers.track(launched_here);
    let lease = app.leases.list(Some(&launched_here), None).await;
    let connect_client = lease[0].client_id.clone();

    socket.close(None).await.unwrap();
    drop(socket);
    wait_for("the launched instance to stop", async || {
        row(&app, launched_here).await.state.is_final()
    })
    .await;
    wait_for("the connect client to end", async || {
        let id = connect_client.clone();
        let stored = app.store.run(move |c| client::get(c, &id)).await.unwrap();

        stored.ended_at.is_some()
    })
    .await;
    app.shutdown().await;
}
