#![allow(clippy::unwrap_used)]

mod support;

use std::{
    io,
    sync::{Arc, Mutex},
};

use brr_core::{ProxyScheme, Secret};
use brr_netproxy::{Forwarder, Upstream};
use reqwest::{Client, Proxy};
use support::{PASSWORD, USER_PW_BASIC};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tracing_subscriber::fmt::MakeWriter;

async fn connect_through(forwarder: &Forwarder, target: std::net::SocketAddr) -> (String, String) {
    let proxy = forwarder.url().replace("http://", "");
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
    let forwarder = Forwarder::start(support::upstream(ProxyScheme::Http, proxy, PASSWORD))
        .await
        .unwrap();
    let client = Client::builder()
        .proxy(Proxy::all(forwarder.url()).unwrap())
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
    let forwarder = Forwarder::start(support::upstream(ProxyScheme::Http, proxy, "wrong"))
        .await
        .unwrap();

    let (head, _) = connect_through(&forwarder, origin).await;

    assert!(head.starts_with("HTTP/1.1 407"), "{head}");
}

#[tokio::test]
async fn socks5_upstream() {
    let (origin, origin_seen) = support::origin("hello").await;
    let (proxy, proxy_seen) = support::socks5_proxy().await;
    let forwarder = Forwarder::start(support::upstream(ProxyScheme::Socks5, proxy, PASSWORD))
        .await
        .unwrap();
    let client = Client::builder()
        .proxy(Proxy::all(forwarder.url()).unwrap())
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

    let wrong = Forwarder::start(support::upstream(ProxyScheme::Socks5, proxy, "wrong"))
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
    let _guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(captured.clone())
            .finish(),
    );
    let (origin, _) = support::origin("hello").await;
    let (socks, _) = support::socks5_proxy().await;
    let (http, _) = support::http_proxy().await;
    let mut formatted = String::new();

    for (scheme, addr) in [(ProxyScheme::Socks5, socks), (ProxyScheme::Http, http)] {
        let upstream = support::upstream(scheme, addr, password);
        let forwarder = Forwarder::start(Arc::clone(&upstream)).await.unwrap();

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

    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    let basic = "dXNlcjpzM2NyM3QtcHc=";

    assert!(logs.contains("forwarder connection failed"), "{logs}");
    assert!(!formatted.contains(password));
    assert!(!logs.contains(password));
    assert!(!logs.contains(basic));
}

#[tokio::test]
async fn dropping_the_forwarder_closes_open_tunnels() {
    let (origin, _) = support::origin("hello").await;
    let (proxy, _) = support::http_proxy().await;
    let forwarder = Forwarder::start(support::upstream(ProxyScheme::Http, proxy, PASSWORD))
        .await
        .unwrap();
    let mut stream = TcpStream::connect(forwarder.url().replace("http://", ""))
        .await
        .unwrap();

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
