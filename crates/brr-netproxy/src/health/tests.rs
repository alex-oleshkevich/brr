use brr_core::ProxyScheme;
use test_case::test_case;

use super::{NO_IP_REASON, check, parse_trace};
use crate::support::{self, PASSWORD};

#[test_case("fl=1\nip=203.0.113.9\nloc=DE\n", Some(("203.0.113.9", Some("DE"))) ; "ip and country")]
#[test_case("ip=2001:db8::1\n", Some(("2001:db8::1", None)) ; "ipv6 without country")]
#[test_case("ip=203.0.113.9\nloc=<script>\n", Some(("203.0.113.9", None)) ; "hostile country")]
#[test_case("loc=DE\n", None ; "no ip")]
#[test_case("ip=not-an-ip\n", None ; "bad ip")]
fn trace(body: &str, expected: Option<(&str, Option<&str>)>) {
    let expected = expected.map(|(ip, country)| (ip.to_owned(), country.map(str::to_owned)));

    assert_eq!(parse_trace(body), expected);
}

#[tokio::test]
async fn healthy_through_http_and_socks5_upstreams() {
    let (origin, _) = support::origin("ip=203.0.113.9\nloc=DE\n").await;
    let (http, _) = support::http_proxy().await;
    let (socks, _) = support::socks5_proxy().await;
    let url = format!("http://{origin}/cdn-cgi/trace");

    for (scheme, addr) in [(ProxyScheme::Http, http), (ProxyScheme::Socks5, socks)] {
        let health = check(support::upstream(scheme, addr, PASSWORD), &url)
            .await
            .unwrap();

        assert_eq!(health.exit_ip, "203.0.113.9");
        assert_eq!(health.country.as_deref(), Some("DE"));
        assert!(health.latency_ms > 0);
    }
}

#[tokio::test]
async fn unhealthy_when_upstream_refuses_or_rejects() {
    let (origin, _) = support::origin("ip=203.0.113.9\n").await;
    let (http, _) = support::http_proxy().await;
    let (socks, _) = support::socks5_proxy().await;
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let url = format!("http://{origin}/");

    for (scheme, addr, password) in [
        (ProxyScheme::Http, closed, PASSWORD),
        (ProxyScheme::Http, http, "wrong"),
        (ProxyScheme::Socks5, socks, "wrong"),
    ] {
        let result = check(support::upstream(scheme, addr, password), &url).await;

        assert!(!result.unwrap_err().is_empty());
    }
}

#[tokio::test]
async fn unhealthy_without_ip_line() {
    let (origin, _) = support::origin("nothing useful\n").await;
    let (http, _) = support::http_proxy().await;
    let result = check(
        support::upstream(ProxyScheme::Http, http, PASSWORD),
        &format!("http://{origin}/"),
    )
    .await;

    assert_eq!(result.unwrap_err(), NO_IP_REASON);
}
