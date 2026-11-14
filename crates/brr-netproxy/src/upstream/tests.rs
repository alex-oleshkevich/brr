use std::{net::Ipv6Addr, sync::Arc};

use brr_core::ProxyScheme;
use tokio::net::TcpListener;

use super::{DialError, Dialer};
use crate::{
    Upstream,
    support::{self, PASSWORD},
};

#[tokio::test]
async fn http_refusal_carries_the_status_code() {
    let (proxy, _) = support::http_proxy().await;
    let dialer = Dialer::new(support::upstream(ProxyScheme::Http, proxy, "wrong")).unwrap();

    let refused = dialer.tunnel("127.0.0.1", 9).await;

    assert!(matches!(refused, Err(DialError::Refused(407))));
}

#[tokio::test]
async fn socks5_rejected_credentials_fail_the_dial() {
    let (proxy, _) = support::socks5_proxy().await;
    let wrong = Dialer::new(support::upstream(ProxyScheme::Socks5, proxy, "wrong")).unwrap();
    let right = Dialer::new(support::upstream(ProxyScheme::Socks5, proxy, PASSWORD)).unwrap();

    // The in-test proxy panics its connection task when the origin is unreachable; the socket drops.
    assert!(matches!(
        wrong.tunnel("127.0.0.1", 9).await,
        Err(DialError::Failed(_))
    ));
    assert!(matches!(
        right.tunnel("127.0.0.1", 1).await,
        Err(DialError::Failed(_))
    ));
}

#[tokio::test]
async fn dials_a_bracketed_ipv6_host() {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream = Arc::new(Upstream {
        scheme: ProxyScheme::Http,
        host: format!("[{}]", addr.ip()),
        port: addr.port(),
        username: None,
        password: None,
    });

    let dialer = Dialer::new(upstream).unwrap();

    let (dialed, accepted) = tokio::join!(dialer.open(), listener.accept());

    dialed.unwrap();
    accepted.unwrap();
}
