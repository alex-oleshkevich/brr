use std::{io, net::IpAddr, sync::Arc};

use base64::{Engine, engine::general_purpose::STANDARD};
use brr_core::{ProxyScheme, Secret};
use rustls::{ClientConfig, pki_types::ServerName};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::TlsConnector;

use crate::{
    DIAL_TIMEOUT, Upstream,
    head::{HeadError, authority, read_head},
};

#[cfg(test)]
mod tests;

// A reply that must not swallow what follows it: a one-byte buffer never reads past the blank line.
const UNBUFFERED: usize = 1;

pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub(crate) type Stream = Box<dyn Io>;

pub(crate) enum DialError {
    // The upstream proxy answered something other than 200; the code is relayed to the browser.
    Refused(u16),
    Failed(io::Error),
}

impl From<io::Error> for DialError {
    fn from(error: io::Error) -> DialError {
        DialError::Failed(error)
    }
}

impl From<HeadError> for DialError {
    fn from(error: HeadError) -> DialError {
        match error {
            HeadError::TooLarge => DialError::Failed(io::Error::other("response head too large")),
            HeadError::Io(error) => DialError::Failed(error),
        }
    }
}

pub(crate) fn basic_auth(upstream: &Upstream) -> Option<String> {
    let username = upstream.username.as_deref()?;
    let password = upstream.password.as_ref().map_or("", Secret::expose);

    Some(format!(
        "Basic {}",
        STANDARD.encode(format!("{username}:{password}"))
    ))
}

pub(crate) struct Dialer {
    pub(crate) upstream: Arc<Upstream>,
    tls: Option<TlsConnector>,
}

impl Dialer {
    // The platform verifier is built once here, so a broken trust store fails the Forwarder at start.
    pub(crate) fn new(upstream: Arc<Upstream>) -> io::Result<Dialer> {
        let tls = if upstream.scheme == ProxyScheme::Https {
            let config = ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .map_err(io::Error::other)?
            .with_platform_verifier()
            .map_err(io::Error::other)?
            .with_no_client_auth();

            Some(TlsConnector::from(Arc::new(config)))
        } else {
            None
        };

        Ok(Dialer { upstream, tls })
    }

    pub(crate) async fn open(&self) -> io::Result<Stream> {
        timeout(DIAL_TIMEOUT, self.connect())
            .await
            .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into()))
    }

    pub(crate) async fn tunnel(&self, host: &str, port: u16) -> Result<Stream, DialError> {
        let dial = async {
            let mut stream = self.connect().await?;

            match self.upstream.scheme {
                ProxyScheme::Socks5 => socks5(&mut stream, &self.upstream, host, port).await?,
                ProxyScheme::Http | ProxyScheme::Https => {
                    http_connect(&mut stream, &self.upstream, host, port).await?
                }
            }

            Ok(stream)
        };

        timeout(DIAL_TIMEOUT, dial)
            .await
            .unwrap_or_else(|_| Err(io::Error::from(io::ErrorKind::TimedOut).into()))
    }

    async fn connect(&self) -> io::Result<Stream> {
        // Stored IPv6 hosts are bracketed, which neither the resolver nor a TLS server name accepts.
        let host = self
            .upstream
            .host
            .trim_start_matches('[')
            .trim_end_matches(']');
        let tcp = TcpStream::connect((host, self.upstream.port)).await?;

        let Some(tls) = &self.tls else {
            return Ok(Box::new(tcp));
        };
        let server_name = ServerName::try_from(host.to_owned()).map_err(io::Error::other)?;

        Ok(Box::new(tls.connect(server_name, tcp).await?))
    }
}

async fn http_connect(
    stream: &mut Stream,
    upstream: &Upstream,
    host: &str,
    port: u16,
) -> Result<(), DialError> {
    let target = authority(host, port);
    let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");

    if let Some(auth) = basic_auth(upstream) {
        request.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await?;

    let response = read_head(&mut BufReader::with_capacity(UNBUFFERED, &mut *stream)).await?;
    let code = response
        .split(|&byte| byte == b' ')
        .nth(1)
        .filter(|code| code.len() == 3)
        .and_then(|code| std::str::from_utf8(code).ok())
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| io::Error::other("upstream answered without a status code"))?;

    if code == 200 {
        return Ok(());
    }

    Err(DialError::Refused(code))
}

async fn socks5(stream: &mut Stream, upstream: &Upstream, host: &str, port: u16) -> io::Result<()> {
    let credentials = upstream.username.as_deref().map(|username| {
        (
            username,
            upstream.password.as_ref().map_or("", Secret::expose),
        )
    });
    let method: u8 = if credentials.is_some() { 0x02 } else { 0x00 };

    stream.write_all(&[5, 1, method]).await?;

    let mut reply = [0; 2];

    stream.read_exact(&mut reply).await?;
    if reply != [5, method] {
        return Err(io::Error::other("socks5 method rejected"));
    }

    if let Some((username, password)) = credentials {
        let mut request = vec![1, field_len(username)?];

        request.extend_from_slice(username.as_bytes());
        request.push(field_len(password)?);
        request.extend_from_slice(password.as_bytes());
        stream.write_all(&request).await?;
        stream.read_exact(&mut reply).await?;
        if reply != [1, 0] {
            return Err(io::Error::other("socks5 credentials rejected"));
        }
    }

    let mut request = vec![5, 1, 0];

    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            request.push(0x01);
            request.extend_from_slice(&ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            request.push(0x04);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            request.extend_from_slice(&[0x03, field_len(host)?]);
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;

    let mut header = [0; 4];

    stream.read_exact(&mut header).await?;
    if header[0] != 5 || header[1] != 0 {
        return Err(io::Error::other(format!("socks5 reply code {}", header[1])));
    }

    let bound_len = match header[3] {
        0x01 => 4,
        0x04 => 16,
        0x03 => usize::from(stream.read_u8().await?),
        _ => return Err(io::Error::other("socks5 reply address type unknown")),
    };

    // Bound address and port are of no use to a forwarder; read them off the stream.
    stream.read_exact(&mut vec![0; bound_len + 2]).await?;

    Ok(())
}

fn field_len(field: &str) -> io::Result<u8> {
    u8::try_from(field.len()).map_err(|_| io::Error::other("socks5 field longer than 255 bytes"))
}
