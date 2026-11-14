use std::{
    io,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use brr_core::{Error, ProxyScheme};
use tokio::{
    io::{self as tio, AsyncWriteExt, BufReader, copy_bidirectional},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout},
};
use tracing::debug;

use crate::{
    Upstream,
    head::{HeadError, authority, read_head},
    upstream::{self, DialError, Dialer},
};

#[cfg(test)]
mod tests;

const ESTABLISHED: &[u8] = b"HTTP/1.1 200 Connection established\r\n\r\n";
const BAD_REQUEST: &[u8] =
    b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
const HEAD_TOO_LARGE: &[u8] =
    b"HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
const BAD_GATEWAY: &[u8] =
    b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";
const HOST_MAX_BYTES: usize = 255;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
const ACCEPT_RETRY: Duration = Duration::from_millis(100);
const MAX_CONNECTIONS: usize = 512;

pub struct Forwarder {
    addr: SocketAddr,
    accept_task: JoinHandle<()>,
}

impl Forwarder {
    pub async fn start(upstream: Arc<Upstream>) -> Result<Forwarder, Error> {
        let bind_failed = |error: io::Error| Error::Internal {
            message: format!("cannot start the forwarding proxy: {error}"),
        };
        let dialer = Arc::new(Dialer::new(upstream).map_err(bind_failed)?);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(bind_failed)?;
        let addr = listener.local_addr().map_err(bind_failed)?;

        Ok(Forwarder {
            addr,
            accept_task: tokio::spawn(accept_loop(listener, dialer)),
        })
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for Forwarder {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

// The JoinSet lives in the accept task, so aborting that task aborts every open tunnel too.
async fn accept_loop(listener: TcpListener, dialer: Arc<Dialer>) {
    let mut connections = JoinSet::new();

    loop {
        tokio::select! {
            accepted = listener.accept(), if connections.len() < MAX_CONNECTIONS => match accepted {
                Ok((client, _)) => {
                    connections.spawn(serve(client, dialer.clone()));
                }
                Err(error) => {
                    debug!(error = format!("{error:#}"), "forwarder accept failed");
                    sleep(ACCEPT_RETRY).await;
                }
            },
            Some(_) = connections.join_next() => {}
        }
    }
}

struct Request<'a> {
    line: &'a str,
    method: &'a str,
    version: &'a str,
    host: String,
    port: u16,
    // None for CONNECT
    path: Option<String>,
}

async fn serve(client: TcpStream, dialer: Arc<Dialer>) {
    // Buffered reads find the head quickly; what they read past it stays in the reader and is
    // forwarded with the rest of the stream.
    let mut client = BufReader::new(client);
    let head = match read_head(&mut client).await {
        Ok(head) => head,
        Err(HeadError::TooLarge) => {
            debug!("forwarder rejected an oversized request head");
            client.write_all(HEAD_TOO_LARGE).await.ok();
            client.shutdown().await.ok();
            // Closing with unread bytes sends a RST that discards the 431 before the browser reads it.
            timeout(DRAIN_TIMEOUT, tio::copy(&mut client, &mut tio::sink()))
                .await
                .ok();
            return;
        }
        Err(HeadError::Io(_)) => return,
    };
    let Some(request) = parse(&head) else {
        debug!("forwarder rejected a malformed request line");
        client.write_all(BAD_REQUEST).await.ok();
        return;
    };
    let destination = authority(&request.host, request.port);

    match handle(&mut client, &dialer, &head, &request).await {
        Ok(()) => debug!(destination, "forwarder connection closed"),
        Err(error) => debug!(
            destination,
            error = format!("{error:#}"),
            "forwarder connection failed"
        ),
    }
}

async fn handle(
    client: &mut BufReader<TcpStream>,
    dialer: &Dialer,
    head: &[u8],
    request: &Request<'_>,
) -> io::Result<()> {
    let socks = dialer.upstream.scheme == ProxyScheme::Socks5;
    let dialed = match (&request.path, socks) {
        (Some(_), false) => dialer.open().await.map_err(DialError::from),
        _ => dialer.tunnel(&request.host, request.port).await,
    };
    let mut remote = match dialed {
        Ok(remote) => remote,
        Err(DialError::Refused(code)) => {
            client
                .write_all(
                    format!("HTTP/1.1 {code} Upstream Proxy Refused\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
                        .as_bytes(),
                )
                .await?;
            return Err(io::Error::other(format!("upstream answered {code}")));
        }
        Err(DialError::Failed(error)) => {
            client.write_all(BAD_GATEWAY).await?;
            return Err(error);
        }
    };

    match &request.path {
        None => client.write_all(ESTABLISHED).await?,
        Some(path) => {
            let (request_line, auth) = if socks {
                (
                    format!("{} {path} {}", request.method, request.version),
                    None,
                )
            } else {
                (
                    request.line.to_owned(),
                    upstream::basic_auth(&dialer.upstream),
                )
            };

            remote
                .write_all(&rewrite_head(head, &request_line, auth.as_deref()))
                .await?;
        }
    }

    copy_bidirectional(client, &mut remote).await?;

    Ok(())
}

// Only the request line is decoded; header values may be Latin-1 and pass through as bytes.
fn parse(head: &[u8]) -> Option<Request<'_>> {
    let line_end = head.windows(2).position(|pair| pair == b"\r\n")?;
    let line = std::str::from_utf8(&head[..line_end]).ok()?;
    let mut parts = line.split(' ');
    let (method, target, version) = (parts.next()?, parts.next()?, parts.next()?);

    if parts.next().is_some() || !version.starts_with("HTTP/1.") {
        return None;
    }

    if method == "CONNECT" {
        let (host, port) = split_authority(target, None)?;

        return Some(Request {
            line,
            method,
            version,
            host,
            port,
            path: None,
        });
    }

    let rest = target.strip_prefix("http://")?;
    let (authority, tail) = rest.split_at(rest.find(['/', '?']).unwrap_or(rest.len()));
    let (host, port) = split_authority(authority, Some(80))?;
    let path = if tail.starts_with('/') {
        tail.to_owned()
    } else {
        format!("/{tail}")
    };

    Some(Request {
        line,
        method,
        version,
        host,
        port,
        path: Some(path),
    })
}

fn split_authority(authority: &str, default_port: Option<u16>) -> Option<(String, u16)> {
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, rest) = bracketed.split_once(']')?;

        (host, rest.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    let port = match port {
        Some(port) => port.parse().ok()?,
        None => default_port?,
    };

    // Hosts end up in log lines.
    let valid = !host.is_empty()
        && host.len() <= HOST_MAX_BYTES
        && host.bytes().all(|byte| byte.is_ascii_graphic());

    valid.then(|| (host.to_owned(), port))
}

fn rewrite_head(head: &[u8], request_line: &str, proxy_auth: Option<&str>) -> Vec<u8> {
    let mut rewritten = format!("{request_line}\r\n").into_bytes();
    let mut lines = head.split(|&byte| byte == b'\n');

    // The request line is replaced; the blank line ends the headers.
    lines.next();
    for line in lines {
        let line = line.strip_suffix(b"\r").unwrap_or(line);

        if line.is_empty() {
            break;
        }

        let name = line.split(|&byte| byte == b':').next().unwrap_or(line);
        let dropped = [
            "proxy-authorization",
            "proxy-connection",
            "connection",
            "keep-alive",
        ]
        .iter()
        .any(|dropped| name.eq_ignore_ascii_case(dropped.as_bytes()));

        if !dropped {
            rewritten.extend_from_slice(line);
            rewritten.extend_from_slice(b"\r\n");
        }
    }

    if let Some(auth) = proxy_auth {
        rewritten.extend_from_slice(format!("Proxy-Authorization: {auth}\r\n").as_bytes());
    }
    rewritten.extend_from_slice(b"Connection: close\r\n\r\n");

    rewritten
}
