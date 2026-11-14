// Each test target uses a different subset of these helpers.
#![allow(dead_code, clippy::unwrap_used)]

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use brr_core::{ProxyScheme, Secret};
use brr_netproxy::Upstream;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream},
};

pub const USER: &str = "user";
pub const PASSWORD: &str = "pw";
pub const USER_PW_BASIC: &str = "Basic dXNlcjpwdw==";

pub type Log<T> = Arc<Mutex<Vec<T>>>;

pub fn upstream(scheme: ProxyScheme, addr: SocketAddr, password: &str) -> Arc<Upstream> {
    Arc::new(Upstream {
        scheme,
        host: addr.ip().to_string(),
        port: addr.port(),
        username: Some(USER.to_owned()),
        password: Some(Secret::new(password)),
    })
}

pub async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();

    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }

    String::from_utf8(head).unwrap()
}

// Records (client peer, request head) per connection and answers `body`.
pub async fn origin(body: &'static str) -> (SocketAddr, Log<(SocketAddr, String)>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Log::default();
    let recorded = seen.clone();

    tokio::spawn(async move {
        loop {
            let (mut client, peer) = listener.accept().await.unwrap();
            let recorded = recorded.clone();

            tokio::spawn(async move {
                let head = read_head(&mut client).await;

                recorded.lock().unwrap().push((peer, head));
                client
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .ok();
                client.shutdown().await.ok();
            });
        }
    });

    (addr, seen)
}

pub struct ProxySeen {
    pub auth: Option<String>,
    // Local address of the connection the proxy opened to the origin.
    pub outbound: Option<SocketAddr>,
}

pub async fn http_proxy() -> (SocketAddr, Log<ProxySeen>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Log::default();
    let recorded = seen.clone();

    tokio::spawn(async move {
        loop {
            let (client, _) = listener.accept().await.unwrap();

            tokio::spawn(serve_http_proxy(client, recorded.clone()));
        }
    });

    (addr, seen)
}

async fn serve_http_proxy(mut client: TcpStream, seen: Log<ProxySeen>) {
    let head = read_head(&mut client).await;
    let auth = head
        .lines()
        .find_map(|line| line.strip_prefix("Proxy-Authorization: "))
        .map(str::to_owned);
    let mut parts = head.lines().next().unwrap().split(' ');
    let (method, target) = (parts.next().unwrap(), parts.next().unwrap());

    if auth.as_deref() != Some(USER_PW_BASIC) {
        seen.lock().unwrap().push(ProxySeen {
            auth,
            outbound: None,
        });
        client
            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n")
            .await
            .ok();
        return;
    }

    let url = target.strip_prefix("http://");
    let authority = url.map_or(target, |url| url.split('/').next().unwrap());
    let mut origin = TcpStream::connect(authority).await.unwrap();

    seen.lock().unwrap().push(ProxySeen {
        auth,
        outbound: Some(origin.local_addr().unwrap()),
    });

    if method == "CONNECT" {
        client.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
    } else {
        let path = &url.unwrap()[authority.len()..];

        origin
            .write_all(
                format!(
                    "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    }

    copy_bidirectional(&mut client, &mut origin).await.ok();
}

// Requires user/pass; Log holds the domain or IP of every successful CONNECT.
pub async fn socks5_proxy() -> (SocketAddr, Log<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Log::default();
    let recorded = seen.clone();

    tokio::spawn(async move {
        loop {
            let (client, _) = listener.accept().await.unwrap();

            tokio::spawn(serve_socks5(client, recorded.clone()));
        }
    });

    (addr, seen)
}

async fn serve_socks5(mut client: TcpStream, seen: Log<String>) {
    let mut greeting = [0; 2];

    client.read_exact(&mut greeting).await.unwrap();
    client
        .read_exact(&mut vec![0; usize::from(greeting[1])])
        .await
        .unwrap();
    client.write_all(&[5, 2]).await.unwrap();

    let _version = client.read_u8().await.unwrap();
    let mut username = vec![0; usize::from(client.read_u8().await.unwrap())];

    client.read_exact(&mut username).await.unwrap();

    let mut password = vec![0; usize::from(client.read_u8().await.unwrap())];

    client.read_exact(&mut password).await.unwrap();

    if username != USER.as_bytes() || password != PASSWORD.as_bytes() {
        client.write_all(&[1, 1]).await.unwrap();
        return;
    }
    client.write_all(&[1, 0]).await.unwrap();

    let mut request = [0; 4];

    client.read_exact(&mut request).await.unwrap();

    let host = match request[3] {
        1 => {
            let mut ip = [0; 4];

            client.read_exact(&mut ip).await.unwrap();
            std::net::Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let mut name = vec![0; usize::from(client.read_u8().await.unwrap())];

            client.read_exact(&mut name).await.unwrap();
            String::from_utf8(name).unwrap()
        }
        atyp => panic!("unexpected address type {atyp}"),
    };
    let port = client.read_u16().await.unwrap();
    let mut origin = TcpStream::connect((host.as_str(), port)).await.unwrap();

    seen.lock().unwrap().push(host);
    client
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .unwrap();
    copy_bidirectional(&mut client, &mut origin).await.ok();
}
