use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

use brr_core::{Error, ProxyScheme};
use tokio::{
    io::{
        self as tio, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf, copy_bidirectional,
    },
    net::{TcpListener, TcpStream, lookup_host},
    sync::mpsc,
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout},
};
use tracing::debug;

use crate::{
    DIAL_TIMEOUT, Upstream,
    head::{HeadError, authority, read_head},
    upstream::{self, DialError, Dialer, Stream},
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
const FORBIDDEN: &[u8] =
    b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";

pub enum Route {
    Upstream(Arc<Upstream>),
    Direct,
}

pub enum Listen {
    Loopback,
    // Serves only the streams handed to `serve`, such as a sandboxed browser's tunnels.
    None,
}

enum Listener {
    Tcp(TcpListener),
    None,
}

impl Listener {
    async fn accept(&self) -> io::Result<Stream> {
        match self {
            Listener::Tcp(listener) => Ok(Box::new(listener.accept().await?.0)),
            Listener::None => std::future::pending().await,
        }
    }
}

// Bytes the browser exchanged with the forwarder, request heads included.
#[derive(Debug, Default)]
pub struct Meter {
    // From the browser.
    pub sent: AtomicU64,
    // To the browser.
    pub received: AtomicU64,
}

// Counts as bytes flow, so a long tunnel shows up before it closes.
struct Counted {
    inner: Stream,
    meter: Arc<Meter>,
}

impl AsyncRead for Counted {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);

        if let Poll::Ready(Ok(())) = polled {
            self.meter
                .sent
                .fetch_add((buf.filled().len() - before) as u64, Ordering::Relaxed);
        }

        polled
    }
}

impl AsyncWrite for Counted {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let polled = Pin::new(&mut self.inner).poll_write(cx, buf);

        if let Poll::Ready(Ok(written)) = polled {
            self.meter
                .received
                .fetch_add(written as u64, Ordering::Relaxed);
        }

        polled
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub struct Forwarder {
    addr: Option<SocketAddr>,
    handed: mpsc::UnboundedSender<Stream>,
    accept_task: JoinHandle<()>,
}

impl Forwarder {
    pub async fn start(
        route: Route,
        listen: Listen,
        meter: Arc<Meter>,
    ) -> Result<Forwarder, Error> {
        let bind_failed = |error: io::Error| Error::Internal {
            message: format!("cannot start the forwarding proxy: {error}"),
        };
        let dialer = match route {
            Route::Upstream(upstream) => {
                Some(Arc::new(Dialer::new(upstream).map_err(bind_failed)?))
            }
            Route::Direct => None,
        };
        let (listener, addr) = match listen {
            Listen::Loopback => {
                let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                    .await
                    .map_err(bind_failed)?;
                let addr = listener.local_addr().map_err(bind_failed)?;

                (Listener::Tcp(listener), Some(addr))
            }
            Listen::None => (Listener::None, None),
        };

        let (handed, handed_rx) = mpsc::unbounded_channel();

        Ok(Forwarder {
            addr,
            handed,
            accept_task: tokio::spawn(accept_loop(listener, handed_rx, dialer, meter)),
        })
    }

    // Hands an already open browser connection to this forwarder, as if it had been accepted.
    pub fn serve<S: AsyncRead + AsyncWrite + Send + Unpin + 'static>(&self, stream: S) {
        // The send only fails once the accept task is gone, and then the stream just closes.
        self.handed.send(Box::new(stream)).ok();
    }

    pub fn url(&self) -> Option<String> {
        self.addr.map(|addr| format!("http://{addr}"))
    }
}

impl Drop for Forwarder {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

// The JoinSet lives in the accept task, so aborting that task aborts every open tunnel too,
// including the handed ones.
async fn accept_loop(
    listener: Listener,
    mut handed: mpsc::UnboundedReceiver<Stream>,
    dialer: Option<Arc<Dialer>>,
    meter: Arc<Meter>,
) {
    let mut connections = JoinSet::new();

    loop {
        let room = connections.len() < MAX_CONNECTIONS;
        let client = tokio::select! {
            accepted = listener.accept(), if room => match accepted {
                Ok(client) => client,
                Err(error) => {
                    debug!(error = format!("{error:#}"), "forwarder accept failed");
                    sleep(ACCEPT_RETRY).await;
                    continue;
                }
            },
            Some(client) = handed.recv(), if room => client,
            Some(_) = connections.join_next() => continue,
        };
        let client = Counted {
            inner: client,
            meter: Arc::clone(&meter),
        };

        connections.spawn(serve(client, dialer.clone()));
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

async fn serve(client: Counted, dialer: Option<Arc<Dialer>>) {
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

    let result = match &dialer {
        Some(dialer) => handle(&mut client, dialer, &head, &request).await,
        None => handle_direct(&mut client, &head, &request).await,
    };

    match result {
        Ok(()) => debug!(destination, "forwarder connection closed"),
        Err(error) => debug!(
            destination,
            error = format!("{error:#}"),
            "forwarder connection failed"
        ),
    }
}

async fn handle(
    client: &mut BufReader<Counted>,
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

async fn handle_direct(
    client: &mut BufReader<Counted>,
    head: &[u8],
    request: &Request<'_>,
) -> io::Result<()> {
    let dialed = timeout(DIAL_TIMEOUT, dial_public(&request.host, request.port))
        .await
        .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into()));
    let mut remote = match dialed {
        Ok(Some(remote)) => remote,
        Ok(None) => {
            client.write_all(FORBIDDEN).await?;
            return Err(io::Error::other("destination is not a public address"));
        }
        Err(error) => {
            client.write_all(BAD_GATEWAY).await?;
            return Err(error);
        }
    };

    match &request.path {
        None => client.write_all(ESTABLISHED).await?,
        Some(path) => {
            let request_line = format!("{} {path} {}", request.method, request.version);

            remote
                .write_all(&rewrite_head(head, &request_line, None))
                .await?;
        }
    }

    copy_bidirectional(client, &mut remote).await?;

    Ok(())
}

// Connects to the first public address and never resolves again, so DNS rebinding cannot swap in a private one.
async fn dial_public(host: &str, port: u16) -> io::Result<Option<TcpStream>> {
    let Some(addr) = lookup_host((host, port))
        .await?
        .find(|addr| is_public(addr.ip()))
    else {
        return Ok(None);
    };

    Ok(Some(TcpStream::connect(addr).await?))
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();

    !(a == 0
        || ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a >= 240
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (18..20).contains(&b)))
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];

    if ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || first & 0xfe00 == 0xfc00
        || first & 0xffc0 == 0xfe80
        || (first == 0x2001 && ip.segments()[1] == 0x0db8)
    {
        return false;
    }

    // Mapped and compatible forms carry an IPv4 address that must pass the IPv4 policy.
    ip.to_ipv4().is_none_or(is_public_v4)
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
