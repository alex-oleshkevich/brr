#![allow(clippy::unwrap_used)]

use crate::common;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use base64::Engine;
use brr_core::{
    Capability, ClientKind, Error, IdleOverrides, Instance, InstanceId, InstanceState, Size, Tab,
    TabId, TabState,
};
use brr_daemon::{App, CallClient, ops};
use brr_engine::page::jpeg_size;
use brr_store::{build, instance, tab};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{net::TcpStream, time};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

use common::{
    KillOnDrop, add_profile, chrome_app, ctx, instance_row, listen, scratch_app, settled, tab_row,
};

const PROFILE: &str = "p";
// The button covers the top left; the input sits below it.
const PAGE: &str = "data:text/html,<body style=\"margin:0\"><button style=\"position:absolute;left:0;top:0;width:200px;height:100px\" onclick=\"document.title='clicked'\">b</button><input id=i style=\"position:absolute;left:0;top:200px\"></body>";
const DIALOG_PAGE: &str = "data:text/html,<body style=\"margin:0\"><input id=i><button style=\"position:absolute;left:0;top:100px;width:200px;height:100px\" onclick=\"alert('hi')\">b</button></body>";
const WAIT: Duration = Duration::from_secs(10);
const FRAME_WAIT: Duration = Duration::from_secs(3);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

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

async fn client(app: &Arc<App>, kind: ClientKind, label: &str) -> CallClient {
    let created = app
        .clients
        .create(app, kind, label.to_string())
        .await
        .unwrap();

    app.clients.resolve(app, &created.id).await.unwrap()
}

async fn app_with_screencast() -> Option<(tempfile::TempDir, Arc<App>, KillOnDrop)> {
    let (dir, app, build_id) = chrome_app(&[]).await?;
    app.store
        .run(move |c| {
            let mut found = build::get(c, &build_id)?;
            found.capabilities = vec![Capability::Screencast];

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
) -> (InstanceId, TabId) {
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
    let tab: Tab = serde_json::from_value(launched["tab"].clone()).unwrap();

    (instance.id, tab.id)
}

async fn connect(addr: SocketAddr, tab_id: TabId, viewer: &CallClient) -> Socket {
    let url = format!(
        "ws://{addr}/v1/tabs/{tab_id}/screencast?client_id={}",
        viewer.client_id
    );
    let (socket, _) = connect_async(url).await.unwrap();

    socket
}

// The next message of `kind`; others are skipped. None when the socket ends first.
async fn next_of(socket: &mut Socket, kind: &str, wait: Duration) -> Option<Value> {
    time::timeout(wait, async {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == kind {
                    return Some(value);
                }
            }
        }

        None
    })
    .await
    .unwrap_or_else(|_| panic!("no {kind} message within {wait:?}"))
}

async fn send(socket: &mut Socket, message: Value) {
    socket
        .send(Message::Text(message.to_string().into()))
        .await
        .unwrap();
}

async fn evaluate(app: &Arc<App>, client: &CallClient, tab_id: TabId, expression: &str) -> Value {
    call(
        app,
        client,
        "tab.evaluate",
        json!({"tab_id": tab_id, "expression": expression}),
    )
    .await
    .unwrap()["result"]
        .clone()
}

// Frame pixels of a CSS point, from the size of the frame itself.
fn frame_point(frame: &Value, css_x: f64, css_y: f64) -> (f64, f64) {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(frame["data"].as_str().unwrap())
        .unwrap();
    let (width, height) = jpeg_size(&bytes).unwrap();
    let metadata = &frame["metadata"];
    let scale = metadata["pageScaleFactor"].as_f64().unwrap();

    (
        css_x * scale * f64::from(width) / metadata["deviceWidth"].as_f64().unwrap(),
        css_y * scale * f64::from(height) / metadata["deviceHeight"].as_f64().unwrap(),
    )
}

#[tokio::test]
async fn unsupported_build_answers_on_the_socket() {
    let (_dir, app) = scratch_app().await;
    let row = instance_row(InstanceState::Running, None);
    let seeded = tab_row(row.id, TabState::Ready);
    let tab_id = seeded.id;
    app.store
        .run(move |c| {
            instance::insert(c, &row)?;
            tab::insert(c, &seeded)
        })
        .await
        .unwrap();
    let u = client(&app, ClientKind::Ui, "ui").await;
    let addr = listen(&app).await;

    let mut socket = connect(addr, tab_id, &u).await;
    let error = next_of(&mut socket, "error", WAIT).await.unwrap();

    assert_eq!(error["code"], "EngineUnsupported");
}

#[tokio::test]
async fn unknown_client_and_tab_are_refused_before_the_upgrade() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let u = client(&app, ClientKind::Ui, "ui").await;

    for url in [
        format!(
            "ws://{addr}/v1/tabs/{}/screencast?client_id={}",
            TabId::new(),
            u.client_id
        ),
        format!(
            "ws://{addr}/v1/tabs/{}/screencast?client_id={}",
            TabId::new(),
            brr_core::ClientId::new()
        ),
    ] {
        let refused = connect_async(url).await;

        assert!(refused.is_err());
    }
}

#[tokio::test]
async fn fan_out_and_input() {
    let Some((_dir, app, browsers)) = app_with_screencast().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let u = client(&app, ClientKind::Ui, "ui").await;
    let v = client(&app, ClientKind::Ui, "ui2").await;
    let (instance_id, tab_id) = launch(&app, &browsers, &a, PAGE).await;
    let addr = listen(&app).await;
    call(
        &app,
        &a,
        "tab.click",
        json!({"tab_id": tab_id, "selector": "#i"}),
    )
    .await
    .unwrap();

    let mut first = connect(addr, tab_id, &u).await;
    let mut second = connect(addr, tab_id, &v).await;
    let frame = next_of(&mut first, "frame", FRAME_WAIT).await.unwrap();
    next_of(&mut second, "frame", FRAME_WAIT).await.unwrap();

    let (x, y) = frame_point(&frame, 100.0, 50.0);
    let click = |event: &str| json!({"type": "mouse", "event": event, "x": x, "y": y, "button": "left", "click_count": 1});
    send(&mut first, click("down")).await;
    let error = next_of(&mut first, "error", WAIT).await.unwrap();
    assert_eq!(error["code"], "LeaseHeld");
    send(&mut first, click("up")).await;
    next_of(&mut first, "error", WAIT).await.unwrap();
    assert_eq!(evaluate(&app, &a, tab_id, "document.title").await, "");

    call(&app, &u, "tab.take_over", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    send(&mut first, click("down")).await;
    send(&mut first, click("up")).await;
    settled(async || evaluate(&app, &u, tab_id, "document.title").await == "clicked").await;

    // The click on the button moved the focus there; the person clicks the input next.
    let (input_x, input_y) = frame_point(&frame, 50.0, 210.0);
    for event in ["down", "up"] {
        send(
            &mut first,
            json!({"type": "mouse", "event": event, "x": input_x, "y": input_y, "button": "left", "click_count": 1}),
        )
        .await;
    }
    for event in ["down", "up"] {
        send(
            &mut first,
            json!({"type": "key", "event": event, "key": "a", "code": "KeyA", "key_code": 65, "text": "a"}),
        )
        .await;
    }
    settled(async || evaluate(&app, &u, tab_id, "document.getElementById('i').value").await == "a")
        .await;

    call(&app, &u, "tab.hand_back", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    call(
        &app,
        &a,
        "instance.stop",
        json!({"instance_id": instance_id}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn awaiting_human_and_dialogs() {
    let Some((_dir, app, browsers)) = app_with_screencast().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let u = client(&app, ClientKind::Ui, "ui").await;
    let (instance_id, tab_id) = launch(&app, &browsers, &a, DIALOG_PAGE).await;
    let addr = listen(&app).await;
    call(
        &app,
        &a,
        "tab.click",
        json!({"tab_id": tab_id, "selector": "#i"}),
    )
    .await
    .unwrap();
    let mut first = connect(addr, tab_id, &u).await;
    let mut second = connect(addr, tab_id, &u).await;
    next_of(&mut first, "frame", FRAME_WAIT).await.unwrap();

    let waiting = tokio::spawn({
        let (app, a) = (Arc::clone(&app), a.clone());
        async move {
            call(
                &app,
                &a,
                "tab.await_human",
                json!({"tab_id": tab_id, "reason": "login", "timeout_ms": 60000}),
            )
            .await
        }
    });
    settled(async || {
        call(&app, &a, "tab.get", json!({"tab_id": tab_id}))
            .await
            .unwrap()["state"]
            == "awaiting_human"
    })
    .await;

    send(&mut first, json!({"type": "text", "text": "hello"})).await;
    settled(async || {
        evaluate(&app, &a, tab_id, "document.getElementById('i').value").await == "hello"
    })
    .await;

    call(
        &app,
        &a,
        "tab.click",
        json!({"tab_id": tab_id, "selector": "button"}),
    )
    .await
    .unwrap();
    for socket in [&mut first, &mut second] {
        let dialog = next_of(socket, "dialog", WAIT).await.unwrap();
        assert_eq!(dialog["dialog_type"], "alert");
        assert_eq!(dialog["message"], "hi");
    }

    send(&mut first, json!({"type": "dialog_answer", "accept": true})).await;
    for socket in [&mut first, &mut second] {
        next_of(socket, "dialog_closed", WAIT).await.unwrap();
    }

    call(&app, &u, "tab.human_done", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    waiting.await.unwrap().unwrap();
    call(
        &app,
        &a,
        "instance.stop",
        json!({"instance_id": instance_id}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn close_and_idle() {
    let Some((_dir, app, browsers)) = app_with_screencast().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let u = client(&app, ClientKind::Ui, "ui").await;
    let (instance_id, first_tab) = launch(&app, &browsers, &a, PAGE).await;
    let second = call(&app, &a, "tab.open", json!({"instance_id": instance_id}))
        .await
        .unwrap();
    let second_tab: TabId = serde_json::from_value(second["tab"]["id"].clone()).unwrap();
    let addr = listen(&app).await;

    // Nobody watching: the browser's screencast stops, so the Tab can start one itself.
    let mut viewer = connect(addr, first_tab, &u).await;
    next_of(&mut viewer, "frame", FRAME_WAIT).await.unwrap();
    drop(viewer);
    let page = app.instances.page(&instance_id, &first_tab).await.unwrap();
    settled(async || {
        page.start_screencast(
            Size {
                width: 320,
                height: 240,
            },
            50,
        )
        .await
        .is_ok()
    })
    .await;

    let mut viewer = connect(addr, second_tab, &u).await;
    next_of(&mut viewer, "frame", FRAME_WAIT).await.unwrap();
    call(&app, &a, "tab.close", json!({"tab_id": second_tab}))
        .await
        .unwrap();
    assert!(next_of(&mut viewer, "closed", WAIT).await.is_some());
    let ended = time::timeout(WAIT, async {
        while let Some(Ok(message)) = viewer.next().await {
            if matches!(message, Message::Close(_)) {
                return;
            }
        }
    })
    .await;
    assert!(ended.is_ok());

    call(
        &app,
        &a,
        "instance.stop",
        json!({"instance_id": instance_id}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_click_that_opens_a_dialog_does_not_freeze_the_view() {
    let Some((_dir, app, browsers)) = app_with_screencast().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let u = client(&app, ClientKind::Ui, "ui").await;
    let (instance_id, tab_id) = launch(&app, &browsers, &a, DIALOG_PAGE).await;
    let addr = listen(&app).await;
    call(&app, &u, "tab.take_over", json!({"tab_id": tab_id}))
        .await
        .unwrap();
    let mut viewer = connect(addr, tab_id, &u).await;
    let frame = next_of(&mut viewer, "frame", FRAME_WAIT).await.unwrap();

    let (x, y) = frame_point(&frame, 100.0, 150.0);
    for event in ["down", "up"] {
        send(
            &mut viewer,
            json!({"type": "mouse", "event": event, "x": x, "y": y, "button": "left", "click_count": 1}),
        )
        .await;
    }

    // The input call that opened the dialog never returns until the dialog is answered, so the
    // dialog message must not wait behind it.
    let dialog = next_of(&mut viewer, "dialog", FRAME_WAIT).await.unwrap();
    assert_eq!(dialog["message"], "hi");
    send(
        &mut viewer,
        json!({"type": "dialog_answer", "accept": true}),
    )
    .await;
    next_of(&mut viewer, "dialog_closed", FRAME_WAIT)
        .await
        .unwrap();

    call(
        &app,
        &u,
        "instance.stop",
        json!({"instance_id": instance_id}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn stopping_the_instance_closes_the_view() {
    let Some((_dir, app, browsers)) = app_with_screencast().await else {
        return;
    };
    let a = client(&app, ClientKind::Rest, "a").await;
    let u = client(&app, ClientKind::Ui, "ui").await;
    let (instance_id, tab_id) = launch(&app, &browsers, &a, PAGE).await;
    let addr = listen(&app).await;
    let mut viewer = connect(addr, tab_id, &u).await;
    next_of(&mut viewer, "frame", FRAME_WAIT).await.unwrap();

    call(
        &app,
        &a,
        "instance.stop",
        json!({"instance_id": instance_id}),
    )
    .await
    .unwrap();

    assert!(next_of(&mut viewer, "closed", WAIT).await.is_some());
}
