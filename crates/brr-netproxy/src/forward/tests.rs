use std::{
    net::IpAddr,
    sync::{Arc, atomic::Ordering},
};

use brr_core::ProxyScheme;
use test_case::test_case;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};

use super::{Forwarder, Listen, Meter, Route, is_public, parse, rewrite_head};
use crate::{HEAD_MAX_BYTES, Upstream, head::read_head};

const DUPLEX_BYTES: usize = 4096;

fn dummy_upstream() -> Arc<Upstream> {
    Arc::new(Upstream {
        scheme: ProxyScheme::Http,
        host: "127.0.0.1".to_owned(),
        port: 1,
        username: None,
        password: None,
    })
}

async fn exchange(forwarder: &Forwarder, request: &[u8]) -> String {
    let mut stream = TcpStream::connect(forwarder.url().unwrap().replace("http://", ""))
        .await
        .unwrap();
    let mut response = String::new();

    stream.write_all(request).await.ok();
    stream.read_to_string(&mut response).await.unwrap();

    response
}

#[tokio::test]
async fn head_cap() {
    let forwarder = Forwarder::start(
        Route::Upstream(dummy_upstream()),
        Listen::Loopback,
        Arc::default(),
    )
    .await
    .unwrap();
    let oversized = vec![b'a'; HEAD_MAX_BYTES + 6 * 1024];

    assert!(
        exchange(&forwarder, &oversized)
            .await
            .starts_with("HTTP/1.1 431")
    );
    assert!(
        exchange(&forwarder, b"nonsense\r\n\r\n")
            .await
            .starts_with("HTTP/1.1 400")
    );
}

#[test_case("127.0.0.1", false ; "loopback")]
#[test_case("10.0.0.1", false ; "private 10")]
#[test_case("172.16.5.4", false ; "private 172")]
#[test_case("192.168.1.1", false ; "private 192")]
#[test_case("169.254.169.254", false ; "link local")]
#[test_case("100.64.0.1", false ; "cgnat")]
#[test_case("::1", false ; "loopback v6")]
#[test_case("fc00::1", false ; "unique local")]
#[test_case("fe80::1", false ; "link local v6")]
#[test_case("::ffff:127.0.0.1", false ; "mapped loopback")]
#[test_case("1.1.1.1", true ; "public v4")]
#[test_case("2606:4700::1111", true ; "public v6")]
fn address_policy(ip: &str, allowed: bool) {
    assert_eq!(is_public(ip.parse::<IpAddr>().unwrap()), allowed);
}

#[tokio::test]
async fn direct_refuses_loopback_and_meters_a_handed_stream() {
    let meter = Arc::new(Meter::default());
    let forwarder = Forwarder::start(Route::Direct, Listen::None, Arc::clone(&meter))
        .await
        .unwrap();
    let (mut stream, handed) = tokio::io::duplex(DUPLEX_BYTES);
    forwarder.serve(handed);

    let request = b"CONNECT 127.0.0.1:9 HTTP/1.1\r\n\r\n";
    let mut response = String::new();
    stream.write_all(request).await.unwrap();
    stream.read_to_string(&mut response).await.unwrap();

    assert_eq!(forwarder.url(), None);
    assert!(response.starts_with("HTTP/1.1 403"));
    assert_eq!(meter.sent.load(Ordering::Relaxed), request.len() as u64);
    assert_eq!(
        meter.received.load(Ordering::Relaxed),
        response.len() as u64
    );
}

#[test_case("CONNECT example.com:443 HTTP/1.1\r\n\r\n", "example.com", 443, None ; "connect")]
#[test_case("CONNECT [::1]:8443 HTTP/1.1\r\n\r\n", "::1", 8443, None ; "connect ipv6")]
#[test_case("GET http://example.com/a?b=1 HTTP/1.1\r\n\r\n", "example.com", 80, Some("/a?b=1") ; "absolute form")]
#[test_case("GET http://example.com:8080 HTTP/1.1\r\n\r\n", "example.com", 8080, Some("/") ; "port without path")]
#[test_case("GET http://example.com?x=1 HTTP/1.1\r\n\r\n", "example.com", 80, Some("/?x=1") ; "query without path")]
fn parse_request(head: &str, host: &str, port: u16, path: Option<&str>) {
    let request = parse(head.as_bytes()).unwrap();

    assert_eq!(request.host, host);
    assert_eq!(request.port, port);
    assert_eq!(request.path.as_deref(), path);
}

#[test_case("CONNECT example.com HTTP/1.1\r\n\r\n" ; "connect without port")]
#[test_case("CONNECT exa\x1bmple.com:443 HTTP/1.1\r\n\r\n" ; "control character in host")]
#[test_case("GET /relative HTTP/1.1\r\n\r\n" ; "origin form")]
#[test_case("GET http://example.com/ HTTP/2\r\n\r\n" ; "wrong version")]
#[test_case("GET http://example.com/ HTTP/1.1 extra\r\n\r\n" ; "extra token")]
fn parse_rejects(head: &str) {
    assert!(parse(head.as_bytes()).is_none());
}

#[test]
fn rewrite_replaces_proxy_and_connection_headers() {
    let head = b"GET http://h/ HTTP/1.1\r\nHost: h\r\nproxy-authorization: Basic old\r\nProxy-Connection: keep-alive\r\nConnection: keep-alive\r\nKeep-Alive: 5\r\nX-Name: caf\xe9\r\n\r\n";

    assert_eq!(
        rewrite_head(head, "GET http://h/ HTTP/1.1", Some("Basic new")),
        b"GET http://h/ HTTP/1.1\r\nHost: h\r\nX-Name: caf\xe9\r\nProxy-Authorization: Basic new\r\nConnection: close\r\n\r\n"
    );
    assert_eq!(
        rewrite_head(head, "GET / HTTP/1.1", None),
        b"GET / HTTP/1.1\r\nHost: h\r\nX-Name: caf\xe9\r\nConnection: close\r\n\r\n"
    );
}

#[tokio::test]
async fn read_head_leaves_the_bytes_after_the_blank_line() {
    const BODY: &[u8] = b"payload";
    let mut input = b"POST http://h/ HTTP/1.1\r\nHost: h\r\n\r\n".to_vec();
    input.extend_from_slice(BODY);
    let mut reader = BufReader::new(input.as_slice());

    let head = read_head(&mut reader).await.ok().unwrap();
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).await.unwrap();

    assert!(head.ends_with(b"\r\n\r\n"));
    assert_eq!(rest, BODY);
}
