#![allow(clippy::unwrap_used)]

use crate::common;

use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};

use brr_core::{ClientKind, EventData, InstanceId, TabId};
use brr_daemon::{App, CallClient, serve};
use brr_store::client;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::oneshot,
    time,
};
use tokio_tungstenite::connect_async;

use common::{boot, listen, scratch_app, settled};

const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(10);
const SOCKET_MODE: u32 = 0o600;
const MODE_BITS: u32 = 0o777;
// Large enough that unread frames fill the socket buffers and then overflow the bus buffer.
const LAG_EVENTS: usize = 4000;
const LAG_PAYLOAD: usize = 16 * 1024;
const EPHEMERAL_TCP: (&str, &str) = ("BRR_SERVER__BIND", "127.0.0.1:0");
const PUBLISHED: i64 = 5;

async fn publish(app: &App, key: &str) {
    app.events
        .publish(EventData::SettingsChanged {
            keys: vec![key.to_string()],
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn routes_from_registry() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let http = reqwest::Client::new();

    for path in ["/v1/settings", "/v1/system/info"] {
        let response = http
            .get(format!("http://{addr}{path}"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 200, "{path}");
        response.json::<Value>().await.unwrap();
    }

    let response = http
        .get(format!("http://{addr}/v1/nope"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(response.json::<Value>().await.unwrap()["code"], "NotFound");
}

#[tokio::test]
async fn error_body_shape() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    let response = reqwest::Client::new()
        .patch(format!("http://{addr}/v1/settings"))
        .json(&json!({"vault.key_source": "file"}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
    let body = response.json::<Value>().await.unwrap();
    assert_eq!(body["code"], "InvalidArgument");
    assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
    assert_eq!(body["details"]["field"], "vault.key_source");
}

#[tokio::test]
async fn sse_replay_then_live() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let base = app.events.last_seq().await;
    for key in ["a", "b", "c"] {
        publish(&app, key).await;
    }

    let mut response = reqwest::Client::new()
        .get(format!("http://{addr}/v1/events/stream"))
        .header("Last-Event-ID", (base + 1).to_string())
        .send()
        .await
        .unwrap();
    // The stream is subscribed once the headers are out, so these two land in replay or live.
    publish(&app, "d").await;
    publish(&app, "e").await;

    let last = base + PUBLISHED;
    let mut text = String::new();
    time::timeout(SETTLE_TIMEOUT, async {
        while !text.contains(&format!("id: {last}\n")) {
            let chunk = response.chunk().await.unwrap().unwrap();
            text.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    })
    .await
    .unwrap();

    let ids: Vec<i64> = text
        .lines()
        .filter_map(|line| line.strip_prefix("id: "))
        .map(|id| id.parse().unwrap())
        .collect();
    assert_eq!(ids, (base + 2..=last).collect::<Vec<_>>());

    let seqs: Vec<i64> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| {
            serde_json::from_str::<Value>(data).unwrap()["seq"]
                .as_i64()
                .unwrap()
        })
        .collect();
    assert_eq!(seqs, ids);
}

#[tokio::test]
async fn sse_first_chunk_without_events() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;

    let mut response = reqwest::get(format!("http://{addr}/v1/events/stream"))
        .await
        .unwrap();
    let chunk = time::timeout(Duration::from_secs(1), response.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert_eq!(&chunk[..], b": connected\n\n");
}

#[tokio::test]
async fn presence_close_ends_client() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let created = app
        .clients
        .create(&app, ClientKind::Cli, "presence".to_string())
        .await
        .unwrap();
    let holder = CallClient {
        client_id: created.id.clone(),
        kind: ClientKind::Cli,
        label: created.label.clone(),
        implicit: false,
    };
    app.leases
        .acquire_tab(&app, &holder, &InstanceId::new(), &TabId::new())
        .await
        .unwrap();

    let (mut socket, _) = connect_async(format!("ws://{addr}/v1/clients/{}/presence", created.id))
        .await
        .unwrap();
    assert_eq!(app.leases.list(None, Some(&created.id)).await.len(), 1);

    socket.close(None).await.unwrap();
    drop(socket);

    settled(async || {
        let id = created.id.clone();
        let stored = app.store.run(move |c| client::get(c, &id)).await.unwrap();

        stored.ended_at.is_some() && app.leases.list(None, Some(&created.id)).await.is_empty()
    })
    .await;
}

#[tokio::test]
async fn serve_unix_and_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::start(boot(dir.path(), &[EPHEMERAL_TCP]))
        .await
        .unwrap();
    let socket = app.paths.socket(&app.settings());
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(serve(Arc::clone(&app), async {
        let _ = stopped.await;
    }));

    let mut stream = time::timeout(SETTLE_TIMEOUT, async {
        loop {
            match UnixStream::connect(&socket).await {
                Ok(stream) => break stream,
                Err(_) => time::sleep(POLL).await,
            }
        }
    })
    .await
    .unwrap();
    stream
        .write_all(b"GET /v1/system/info HTTP/1.1\r\nHost: brr\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();

    let mode = std::fs::metadata(&socket).unwrap().permissions().mode() & MODE_BITS;
    assert_eq!(mode, SOCKET_MODE);
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    let (_, body) = reply.split_once("\r\n\r\n").unwrap();
    serde_json::from_str::<Value>(body).unwrap();

    stop.send(()).unwrap();
    time::timeout(SETTLE_TIMEOUT, server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!socket.exists());
}

#[tokio::test]
async fn sse_lagged_ends_the_stream() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let mut response = reqwest::Client::new()
        .get(format!("http://{addr}/v1/events/stream"))
        .send()
        .await
        .unwrap();

    // Transient events skip the database, so the burst outruns the unread connection.
    for _ in 0..LAG_EVENTS {
        app.events
            .publish(EventData::TabNavigated {
                tab_id: TabId::new(),
                instance_id: InstanceId::new(),
                url: String::new(),
                title: "x".repeat(LAG_PAYLOAD),
            })
            .await
            .unwrap();
    }

    let mut tail = String::new();
    time::timeout(SETTLE_TIMEOUT, async {
        while let Some(chunk) = response.chunk().await.unwrap() {
            tail.push_str(&String::from_utf8_lossy(&chunk));
            let keep = tail.len().saturating_sub(LAG_PAYLOAD);
            tail.drain(..tail.floor_char_boundary(keep));
        }
    })
    .await
    .unwrap();

    assert!(
        tail.trim_end().ends_with("event: lagged\ndata: lagged"),
        "{tail}"
    );
}

#[tokio::test]
async fn sse_cursor_from_the_future_counts_as_now() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let base = app.events.last_seq().await;
    publish(&app, "old").await;

    let mut response = reqwest::Client::new()
        .get(format!("http://{addr}/v1/events/stream"))
        .header("Last-Event-ID", (base + 1_000).to_string())
        .send()
        .await
        .unwrap();
    publish(&app, "new").await;

    let newest = base + 2;
    let mut text = String::new();
    time::timeout(SETTLE_TIMEOUT, async {
        while !text.contains(&format!("id: {newest}\n")) {
            let chunk = response.chunk().await.unwrap().unwrap();
            text.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    })
    .await
    .unwrap();

    assert!(!text.contains(&format!("id: {}\n", base + 1)), "{text}");
}
