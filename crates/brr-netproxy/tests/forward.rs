#![allow(clippy::unwrap_used)]

mod support;

use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use brr_core::{ProxyScheme, Secret};
use brr_netproxy::{Forwarder, Listen, Meter, Route, Upstream};
use reqwest::{Client, Proxy};
use support::{PASSWORD, USER_PW_BASIC};
use test_case::test_case;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, duplex},
    net::TcpStream,
};
use tracing_subscriber::fmt::MakeWriter;

const LOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const LOG_POLL: std::time::Duration = std::time::Duration::from_millis(10);
const BODY_BYTES: usize = 256 * 1024;
const DUPLEX_BYTES: usize = 16 * 1024;

trait Browser: AsyncRead + AsyncWrite + Unpin {}

impl<T: AsyncRead + AsyncWrite + Unpin> Browser for T {}

// The browser's end of a connection the forwarder accepted, or of one handed to it in memory.
async fn open(forwarder: &Forwarder, handed: bool) -> Box<dyn Browser> {
    if handed {
        let (browser, proxy_end) = duplex(DUPLEX_BYTES);

        forwarder.serve(proxy_end);
        Box::new(browser)
    } else {
        Box::new(
            TcpStream::connect(forwarder.url().unwrap().replace("http://", ""))
                .await
                .unwrap(),
        )
    }
}

fn load(counter: &AtomicU64) -> usize {
    usize::try_from(counter.load(Ordering::Relaxed)).unwrap()
}

async fn connect_through(forwarder: &Forwarder, target: std::net::SocketAddr) -> (String, String) {
    let proxy = forwarder.url().unwrap().replace("http://", "");
    let mut stream = TcpStream::connect(proxy).await.unwrap();

    stream
        .write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .unwrap();

    let head = support::read_head(&mut stream).await;

    if !head.contains(" 200 ") {
        return (head, String::new());
    }

    stream
        .write_all(
            format!("GET /tunneled HTTP/1.1\r\nHost: {target}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();

    let mut body = String::new();

    stream.read_to_string(&mut body).await.unwrap();

    (head, body)
}

#[tokio::test]
async fn http_upstream_auth() {
    let (origin, origin_seen) = support::origin("hello").await;
    let (proxy, proxy_seen) = support::http_proxy().await;
    let forwarder = Forwarder::start(
        Route::Upstream(support::upstream(ProxyScheme::Http, proxy, PASSWORD)),
        Listen::Loopback,
        Arc::default(),
    )
    .await
    .unwrap();
    let client = Client::builder()
        .proxy(Proxy::all(forwarder.url().unwrap()).unwrap())
        .build()
        .unwrap();

    let body = client
        .get(format!("http://{origin}/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let (head, tunneled) = connect_through(&forwarder, origin).await;

    assert_eq!(body, "hello");
    assert!(head.starts_with("HTTP/1.1 200"));
    assert!(tunneled.ends_with("hello"));

    let proxy_seen = proxy_seen.lock().unwrap();
    let origin_seen = origin_seen.lock().unwrap();

    assert_eq!(proxy_seen.len(), 2);
    assert_eq!(origin_seen.len(), 2);
    for (from_proxy, at_origin) in proxy_seen.iter().zip(origin_seen.iter()) {
        assert_eq!(from_proxy.auth.as_deref(), Some(USER_PW_BASIC));
        assert_eq!(from_proxy.outbound, Some(at_origin.0));
    }
    assert!(origin_seen[0].1.starts_with("GET / HTTP/1.1\r\n"));
    assert!(origin_seen[1].1.starts_with("GET /tunneled HTTP/1.1\r\n"));
}

#[tokio::test]
async fn http_upstream_rejects_wrong_password() {
    let (origin, _) = support::origin("hello").await;
    let (proxy, _) = support::http_proxy().await;
    let forwarder = Forwarder::start(
        Route::Upstream(support::upstream(ProxyScheme::Http, proxy, "wrong")),
        Listen::Loopback,
        Arc::default(),
    )
    .await
    .unwrap();

    let (head, _) = connect_through(&forwarder, origin).await;

    assert!(head.starts_with("HTTP/1.1 407"), "{head}");
}

#[tokio::test]
async fn socks5_upstream() {
    let (origin, origin_seen) = support::origin("hello").await;
    let (proxy, proxy_seen) = support::socks5_proxy().await;
    let forwarder = Forwarder::start(
        Route::Upstream(support::upstream(ProxyScheme::Socks5, proxy, PASSWORD)),
        Listen::Loopback,
        Arc::default(),
    )
    .await
    .unwrap();
    let client = Client::builder()
        .proxy(Proxy::all(forwarder.url().unwrap()).unwrap())
        .build()
        .unwrap();

    let (head, tunneled) = connect_through(&forwarder, origin).await;
    let body = client
        .get(format!("http://localhost:{}/plain", origin.port()))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(head.starts_with("HTTP/1.1 200"));
    assert!(tunneled.ends_with("hello"));
    assert_eq!(body, "hello");
    assert_eq!(*proxy_seen.lock().unwrap(), ["127.0.0.1", "localhost"]);

    {
        let origin_seen = origin_seen.lock().unwrap();

        assert!(origin_seen[0].1.starts_with("GET /tunneled HTTP/1.1\r\n"));
        // Origin-form, not absolute-form, once the socks tunnel is open.
        assert!(origin_seen[1].1.starts_with("GET /plain HTTP/1.1\r\n"));
    }

    let wrong = Forwarder::start(
        Route::Upstream(support::upstream(ProxyScheme::Socks5, proxy, "wrong")),
        Listen::Loopback,
        Arc::default(),
    )
    .await
    .unwrap();
    let (head, _) = connect_through(&wrong, origin).await;

    assert!(head.starts_with("HTTP/1.1 502"), "{head}");
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);

        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Captured {
    type Writer = Captured;

    fn make_writer(&self) -> Captured {
        self.clone()
    }
}

#[tokio::test]
async fn secret_never_logged() {
    let password = "s3cr3t-pw";
    let captured = Captured::default();
    // Global, not scoped: a thread-local subscriber races with sibling tests over the callsites'
    // cached interest. No other test in this binary sets a subscriber.
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(captured.clone())
            .finish(),
    )
    .unwrap();
    let (origin, _) = support::origin("hello").await;
    let (socks, _) = support::socks5_proxy().await;
    let (http, _) = support::http_proxy().await;
    let mut formatted = String::new();

    for (scheme, addr) in [(ProxyScheme::Socks5, socks), (ProxyScheme::Http, http)] {
        let upstream = support::upstream(scheme, addr, password);
        let forwarder = Forwarder::start(
            Route::Upstream(Arc::clone(&upstream)),
            Listen::Loopback,
            Arc::default(),
        )
        .await
        .unwrap();

        formatted.push_str(&format!("{upstream:?}"));
        connect_through(&forwarder, origin).await;
    }

    let upstream = Upstream {
        scheme: ProxyScheme::Http,
        host: "h".to_owned(),
        port: 1,
        username: Some("user".to_owned()),
        password: Some(Secret::new(password)),
    };

    formatted.push_str(&format!("{upstream:?}"));

    // The forwarder logs the failed connection from its own task, after the client side returns.
    let logs = tokio::time::timeout(LOG_TIMEOUT, async {
        loop {
            let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
            if logs.contains("forwarder connection failed") {
                return logs;
            }
            tokio::time::sleep(LOG_POLL).await;
        }
    })
    .await
    .unwrap_or_else(|_| String::from_utf8(captured.0.lock().unwrap().clone()).unwrap());
    let basic = "dXNlcjpzM2NyM3QtcHc=";

    assert!(logs.contains("forwarder connection failed"), "{logs}");
    assert!(!formatted.contains(password));
    assert!(!logs.contains(password));
    assert!(!logs.contains(basic));
}

#[test_case(false ; "accepted")]
#[test_case(true ; "handed")]
#[tokio::test]
async fn dropping_the_forwarder_closes_open_tunnels(handed: bool) {
    let (origin, _) = support::origin("hello").await;
    let (proxy, _) = support::http_proxy().await;
    let forwarder = Forwarder::start(
        Route::Upstream(support::upstream(ProxyScheme::Http, proxy, PASSWORD)),
        Listen::Loopback,
        Arc::default(),
    )
    .await
    .unwrap();
    let mut stream = open(&forwarder, handed).await;

    stream
        .write_all(format!("CONNECT {origin} HTTP/1.1\r\n\r\n").as_bytes())
        .await
        .unwrap();
    support::read_head(&mut stream).await;
    drop(forwarder);

    let mut rest = Vec::new();

    assert!(
        stream
            .read_to_end(&mut rest)
            .await
            .map_or(true, |_| rest.is_empty())
    );
}

#[test_case(false ; "accepted")]
#[test_case(true ; "handed")]
#[tokio::test]
async fn meters_everything_the_browser_exchanges(handed: bool) {
    let body: &'static str = "x".repeat(BODY_BYTES).leak();
    let (origin, _) = support::origin(body).await;
    let (proxy, _) = support::http_proxy().await;
    let meter = Arc::new(Meter::default());
    let forwarder = Forwarder::start(
        Route::Upstream(support::upstream(ProxyScheme::Http, proxy, PASSWORD)),
        Listen::Loopback,
        Arc::clone(&meter),
    )
    .await
    .unwrap();
    let mut browser = open(&forwarder, handed).await;
    // The tunneled request follows the CONNECT head at once, as a pipelining browser would send it.
    let request = format!(
        "CONNECT {origin} HTTP/1.1\r\n\r\nGET / HTTP/1.1\r\nHost: {origin}\r\nConnection: close\r\n\r\n"
    );
    let mut response = Vec::new();

    browser.write_all(request.as_bytes()).await.unwrap();
    browser.read_to_end(&mut response).await.unwrap();

    assert!(response.starts_with(b"HTTP/1.1 200 Connection established\r\n\r\n"));
    assert!(response.ends_with(body.as_bytes()));
    assert_eq!(load(&meter.sent), request.len());
    assert_eq!(load(&meter.received), response.len());
    assert!(load(&meter.received) > BODY_BYTES);
}
