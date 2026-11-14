#![allow(clippy::unwrap_used, clippy::disallowed_macros)]

use crate::common;

use std::{
    io::Read,
    sync::{Arc, Mutex},
    time::Duration,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use brr_core::{
    AdmissionMode, BuildId, Capability, ConflictReason, Error, InstanceId, InstanceState, TabState,
};
use brr_daemon::{App, ops, services::instances::LaunchRequest};
use brr_store::build;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{sync::mpsc, task::JoinHandle, time};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Error as WsError, Message, client::IntoClientRequest, http::Request},
};

use common::{KillOnDrop, add_profile, chrome_app, ctx, listen, row, scratch_app, wait_for};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.6478.55 Safari/537.36";
const EXTENSION_ORIGIN: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop";
const ATTACH_WAIT: Duration = Duration::from_secs(10);

fn request(addr: std::net::SocketAddr, origin: Option<&str>) -> Request<()> {
    let mut request = format!("ws://{addr}/v1/extension-relay")
        .into_client_request()
        .unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
    }

    request
}

enum Control {
    Send(Value),
    Close,
}

struct FakeExtension {
    log: Arc<Mutex<Vec<Value>>>,
    control: mpsc::Sender<Control>,
    _task: JoinHandle<()>,
}

impl FakeExtension {
    async fn connect(addr: std::net::SocketAddr, tabs: &[(u64, &str)]) -> FakeExtension {
        let (socket, _) = connect_async(request(addr, Some(EXTENSION_ORIGIN)))
            .await
            .unwrap();
        let (mut sink, mut stream) = socket.split();
        let send = |value: Value| Message::Text(value.to_string().into());

        sink.send(send(json!({
            "method": "extension.hello",
            "params": [{ "protocol": 1, "version": "0.1.0", "user_agent": USER_AGENT }],
        })))
        .await
        .unwrap();
        for (id, url) in tabs {
            sink.send(send(json!({
                "method": "chrome.tabs.onCreated",
                "params": [{ "id": id, "url": url, "title": format!("tab {id}") }],
            })))
            .await
            .unwrap();
        }
        sink.send(send(
            json!({ "method": "extension.initialized", "params": [] }),
        ))
        .await
        .unwrap();

        let log = Arc::new(Mutex::new(Vec::new()));
        let (control, mut commands) = mpsc::channel(16);
        let seen = Arc::clone(&log);
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    message = stream.next() => {
                        let Some(Ok(Message::Text(text))) = message else { return };
                        let command: Value = serde_json::from_str(&text).unwrap();
                        seen.lock().unwrap().push(command.clone());
                        let reply = json!({ "id": command["id"], "result": answer(&command) });
                        if sink.send(send(reply)).await.is_err() {
                            return;
                        }
                    }
                    control = commands.recv() => match control {
                        Some(Control::Send(value)) => sink.send(send(value)).await.unwrap(),
                        Some(Control::Close) | None => {
                            let _ = sink.close().await;
                            return;
                        }
                    },
                }
            }
        });

        FakeExtension {
            log,
            control,
            _task: task,
        }
    }

    async fn send(&self, value: Value) {
        self.control.send(Control::Send(value)).await.unwrap();
    }

    async fn close(&self) {
        self.control.send(Control::Close).await.unwrap();
    }

    fn methods(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .map(|command| match command["method"].as_str().unwrap() {
                "chrome.debugger.sendCommand" => {
                    format!("sendCommand {}", command["params"][1].as_str().unwrap())
                }
                other => other.to_owned(),
            })
            .collect()
    }

    fn find(&self, method: &str) -> Value {
        self.log
            .lock()
            .unwrap()
            .iter()
            .find(|command| command["method"] == method)
            .unwrap()
            .clone()
    }
}

// What chrome.debugger answers for the commands a Tab sends while it initializes.
fn answer(command: &Value) -> Value {
    if command["method"] != "chrome.debugger.sendCommand" {
        return Value::Null;
    }

    match command["params"][1].as_str().unwrap() {
        "Page.getFrameTree" => json!({ "frameTree": { "frame": {
            "id": "F1", "loaderId": "L1", "url": "https://a.test/", "securityOrigin": "https://a.test",
            "mimeType": "text/html",
        }}}),
        "Page.createIsolatedWorld" => json!({ "executionContextId": 7 }),
        "Runtime.evaluate" => json!({ "result": { "type": "number", "value": 2 } }),
        _ => json!({}),
    }
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

async fn attach_op(app: &Arc<App>) -> Result<Value, Error> {
    ops::dispatch(
        ctx(app),
        ops::find("instance.attach").unwrap(),
        json!({ "via": "extension" }),
    )
    .await
}

async fn attached(app: &Arc<App>, addr: std::net::SocketAddr) -> (FakeExtension, Value) {
    let waiting = tokio::spawn({
        let app = Arc::clone(app);
        async move { attach_op(&app).await }
    });
    time::sleep(Duration::from_millis(100)).await;
    let fake =
        FakeExtension::connect(addr, &[(1, "https://a.test/"), (2, "https://b.test/")]).await;
    let attached = time::timeout(ATTACH_WAIT, waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    (fake, attached)
}

fn instance_id(attached: &Value) -> InstanceId {
    attached["instance"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn attach_via_extension() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    let (_fake, attached) = attached(&app, addr).await;

    assert_eq!(attached["instance"]["kind"], "live");
    assert_eq!(attached["instance"]["state"], "attached");
    let tabs = attached["tabs"].as_array().unwrap();
    assert_eq!(tabs.len(), 2);
    assert!(tabs.iter().all(|tab| tab["opened_by_brr"] == false));
}

#[tokio::test]
async fn command_translation() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let (fake, attached) = attached(&app, addr).await;
    let tab_id = attached["tabs"][0]["id"].clone();

    let evaluated = ops::dispatch(
        ctx(&app),
        ops::find("tab.evaluate").unwrap(),
        json!({ "tab_id": tab_id, "expression": "1+1" }),
    )
    .await
    .unwrap();

    assert_eq!(evaluated["result"], 2);
    let methods = fake.methods();
    let attach = methods
        .iter()
        .position(|method| method == "chrome.debugger.attach")
        .unwrap();
    let evaluate = methods
        .iter()
        .position(|method| method == "sendCommand Runtime.evaluate")
        .unwrap();
    assert!(attach < evaluate, "{methods:?}");
    assert_eq!(
        fake.find("chrome.debugger.attach")["params"],
        json!([{ "tabId": 1 }, "1.3"])
    );
    let sent = fake
        .log
        .lock()
        .unwrap()
        .iter()
        .find(|command| command["params"][1] == "Runtime.evaluate")
        .unwrap()
        .clone();
    assert_eq!(sent["params"][0], json!({ "tabId": 1 }));
}

#[tokio::test]
async fn detach_and_lost() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let (fake, attached) = attached(&app, addr).await;
    let instance_id = instance_id(&attached);

    fake.send(json!({
        "method": "chrome.debugger.onDetach",
        "params": [{ "tabId": 1 }, "canceled_by_user"],
    }))
    .await;
    wait_for("tab 1 to close", async || {
        app.instances.view(&instance_id).is_some_and(|view| {
            view.tabs
                .iter()
                .all(|tab| tab.url != "https://a.test/" || tab.state == TabState::Closed)
        })
    })
    .await;

    fake.close().await;
    wait_for("the instance to be lost", async || {
        row(&app, instance_id).await.state == InstanceState::Lost
    })
    .await;
}

#[tokio::test]
async fn wait_times_out() {
    let (_dir, app) = scratch_app().await;

    time::pause();
    let error = attach_op(&app).await.unwrap_err();

    assert_eq!(
        error,
        Error::Conflict {
            reason: ConflictReason::ExtensionUnanswered,
            detail: None,
        }
    );
}

#[tokio::test]
async fn companion_zip() {
    let (_dir, app) = scratch_app().await;

    let output = ops::dispatch(
        ctx(&app),
        ops::find("extension.companion").unwrap(),
        json!({}),
    )
    .await
    .unwrap();

    assert_eq!(output["filename"], "brr-companion.zip");
    let bytes = STANDARD.decode(output["file"].as_str().unwrap()).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    assert!(archive.by_name("manifest.json").is_ok());
    let mut config = String::new();
    archive
        .by_name("brr.json")
        .unwrap()
        .read_to_string(&mut config)
        .unwrap();
    let config: Value = serde_json::from_str(&config).unwrap();
    let port = app.settings().server.bind.port();
    assert_eq!(
        config["relay_url"],
        format!("ws://127.0.0.1:{port}/v1/extension-relay")
    );
}

#[tokio::test]
async fn companion_profile() {
    let Some((_dir, app, build_id)) = chrome_app(&[]).await else {
        return;
    };
    grant(&app, build_id, vec![Capability::UnpackedExtensions]).await;
    let mut profile = add_profile(&app, build_id, "companion", false, Default::default()).await;
    profile.settings.companion = true;
    let stored = profile.clone();
    app.store
        .run(move |c| brr_store::profile::update(c, &stored))
        .await
        .unwrap();

    let (running, _) = app
        .instances
        .launch(
            &app,
            LaunchRequest {
                profile: profile.clone(),
                display: None,
                priority: 0,
                admission: AdmissionMode::Queue,
                force: false,
            },
        )
        .await
        .unwrap();
    let _guard = KillOnDrop::new(&app, vec![running.id]);

    let loaded: Vec<_> = std::fs::read_dir(app.paths.data_dir.join("companion"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].path().join("brr.json").exists());

    app.store
        .run(move |c| {
            let mut found = build::get(c, &build_id)?;
            found
                .capabilities
                .retain(|c| *c != Capability::UnpackedExtensions);

            build::update(c, &found)
        })
        .await
        .unwrap();
    let error = ops::dispatch(
        ctx(&app),
        ops::find("profile.update").unwrap(),
        json!({ "profile": "companion", "settings": profile.settings }),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error,
        Error::EngineUnsupported {
            capability: Capability::UnpackedExtensions
        }
    );
}

#[tokio::test]
async fn relay_accepts_only_extension_origins() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    for origin in [
        Some("https://evil.test"),
        Some("http://127.0.0.1:9898"),
        None,
    ] {
        let refused = connect_async(request(addr, origin)).await.unwrap_err();
        let WsError::Http(response) = refused else {
            panic!("expected an HTTP refusal for {origin:?}: {refused:?}");
        };
        assert_eq!(response.status(), 403, "{origin:?}");
    }

    let (socket, _) = connect_async(request(addr, Some("moz-extension://1234")))
        .await
        .unwrap();
    drop(socket);
    let _extension = FakeExtension::connect(addr, &[(1, "https://a.test/")]).await;
}
