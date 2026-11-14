use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use brr_core::{ConflictReason, Error};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use super::{connect_ws, resolve_http_endpoint};

const PRODUCT: &str = "X";
const BIG_BYTES: usize = 3 * 1024 * 1024;
const DEBUGGER_URL: &str = "ws://h/devtools/browser/1";

async fn bind() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    (listener, addr)
}

async fn serve_cdp(listener: TcpListener) {
    let (stream, _) = listener.accept().await.unwrap();
    let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();

    while let Some(Ok(Message::Text(text))) = socket.next().await {
        let request: Value = serde_json::from_str(&text).unwrap();
        let result = match request["method"].as_str().unwrap() {
            "Browser.getVersion" => json!({ "product": PRODUCT }),
            _ => json!({ "blob": "a".repeat(BIG_BYTES) }),
        };
        let reply = json!({ "id": request["id"], "result": result });
        socket.send(Message::text(reply.to_string())).await.unwrap();
    }
}

#[tokio::test]
async fn round_trip() {
    let (listener, addr) = bind().await;
    tokio::spawn(serve_cdp(listener));

    let (cdp, _events) = connect_ws(&format!("ws://{addr}/devtools/browser/1"))
        .await
        .unwrap();

    assert_eq!(cdp.browser_version().await.unwrap(), PRODUCT);
    let big = cdp.call(None, "Test.big", json!({})).await.unwrap();
    assert_eq!(big["blob"].as_str().unwrap().len(), BIG_BYTES);
}

#[tokio::test]
async fn connect_errors() {
    let (listener, addr) = bind().await;
    let forbidden = tokio::spawn(async move {
        let app = Router::new().fallback(|| async { StatusCode::FORBIDDEN });
        axum::serve(listener, app).await.unwrap();
    });
    let declined = connect_ws(&format!("ws://{addr}/")).await;
    assert_eq!(
        declined.err(),
        Some(Error::Conflict {
            reason: ConflictReason::AttachDeclined,
            detail: None
        })
    );
    forbidden.abort();

    // Paused only here: auto-advance would fire the 30 s timer while real handshakes are pending.
    tokio::time::pause();
    let (silent, addr) = bind().await;
    let accept = tokio::spawn(async move {
        let _held = silent.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let unanswered = connect_ws(&format!("ws://{addr}/")).await;
    assert_eq!(
        unanswered.err(),
        Some(Error::Conflict {
            reason: ConflictReason::AttachUnanswered,
            detail: None
        })
    );
    accept.abort();

    let (closed, addr) = bind().await;
    drop(closed);
    let refused = connect_ws(&format!("ws://user:pw@{addr}/?token=secret")).await;
    let Some(Error::InvalidArgument { field, message }) = refused.err() else {
        panic!("expected InvalidArgument");
    };
    assert_eq!(field, "cdp_url");
    assert!(!message.contains("secret") && !message.contains("pw@"));
}

#[tokio::test]
async fn eof_during_handshake_is_declined() {
    let (listener, addr) = bind().await;
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        drop(stream);
    });

    let declined = connect_ws(&format!("ws://{addr}/")).await;

    assert_eq!(
        declined.err(),
        Some(Error::Conflict {
            reason: ConflictReason::AttachDeclined,
            detail: None
        })
    );
}

#[tokio::test]
async fn dropping_the_client_closes_the_socket() {
    let (listener, addr) = bind().await;
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(Ok(message)) = socket.next().await {
            if message.is_close() {
                break;
            }
        }
    });
    let (cdp, events) = connect_ws(&format!("ws://{addr}/")).await.unwrap();

    drop((cdp, events));

    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn resolve_http() {
    let (listener, addr) = bind().await;
    let app = Router::new().route(
        "/json/version",
        get(|| async { axum::Json(json!({ "webSocketDebuggerUrl": DEBUGGER_URL })) }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let http = reqwest::Client::new();

    let found = resolve_http_endpoint(&http, &format!("http://{addr}")).await;
    assert_eq!(found.unwrap(), DEBUGGER_URL);
    let passthrough = resolve_http_endpoint(&http, DEBUGGER_URL).await;
    assert_eq!(passthrough.unwrap(), DEBUGGER_URL);

    let (closed, addr) = bind().await;
    drop(closed);
    let Err(Error::InvalidArgument { message, .. }) =
        resolve_http_endpoint(&http, &format!("http://{addr}/?token=secret")).await
    else {
        panic!("expected InvalidArgument");
    };
    assert!(!message.contains("secret"));

    let (listener, addr) = bind().await;
    let app = Router::new().fallback(|| async { StatusCode::NOT_FOUND });
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let Err(Error::InvalidArgument { field, message }) =
        resolve_http_endpoint(&http, &format!("http://{addr}")).await
    else {
        panic!("expected InvalidArgument");
    };
    assert_eq!(field, "cdp_url");
    assert!(message.contains("approval mode"));
}
