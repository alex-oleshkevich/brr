use std::{net::IpAddr, sync::Arc, time::Instant};

use reqwest::{Client, Proxy};
use tracing::debug;

use crate::{CHECK_TIMEOUT, Forwarder, Upstream};

#[cfg(test)]
mod tests;

const BODY_MAX_BYTES: usize = 64 * 1024;
const NO_IP_REASON: &str = "the check page did not report an exit IP";

fn failure_reason(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "timed out waiting for the proxy".to_owned()
    } else if let Some(status) = error.status() {
        format!("the check page answered HTTP {}", status.as_u16())
    } else if error.is_connect() {
        "could not connect through the proxy (check the address, port and login)".to_owned()
    } else {
        "the proxy did not return a usable answer".to_owned()
    }
}

#[derive(Debug)]
pub struct Health {
    pub exit_ip: String,
    pub country: Option<String>,
    pub latency_ms: u64,
}

// The Err is one plain sentence for the person looking at the proxy list.
pub async fn check(upstream: Arc<Upstream>, check_url: &str) -> Result<Health, String> {
    let forwarder = Forwarder::start(upstream)
        .await
        .map_err(|error| format!("could not start the local forwarder: {error}"))?;
    let unhealthy = |error: reqwest::Error| {
        let reason = failure_reason(&error);
        debug!(
            error = format!("{:#}", error.without_url()),
            "proxy health check failed"
        );

        reason
    };
    let client = Client::builder()
        .proxy(Proxy::all(forwarder.url()).map_err(unhealthy)?)
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(unhealthy)?;

    let started = Instant::now();
    let mut response = client
        .get(check_url)
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(unhealthy)?;
    let mut body = Vec::new();

    while body.len() < BODY_MAX_BYTES {
        match response.chunk().await.map_err(unhealthy)? {
            Some(chunk) => body.extend_from_slice(&chunk),
            None => break,
        }
    }

    // A sub-millisecond answer is still a measured latency.
    let latency_ms = u64::try_from(started.elapsed().as_millis())
        .unwrap_or(u64::MAX)
        .max(1);
    let (exit_ip, country) = parse_trace(&String::from_utf8_lossy(&body)).ok_or_else(|| {
        debug!("proxy health check answer has no ip= line");
        NO_IP_REASON.to_owned()
    })?;

    Ok(Health {
        exit_ip,
        country,
        latency_ms,
    })
}

fn parse_trace(body: &str) -> Option<(String, Option<String>)> {
    let mut exit_ip = None;
    let mut country = None;

    for line in body.lines() {
        if let Some(value) = line.strip_prefix("ip=") {
            exit_ip = value.trim().parse::<IpAddr>().ok();
        } else if let Some(value) = line.strip_prefix("loc=") {
            let value = value.trim();

            country = (value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_alphabetic()))
                .then(|| value.to_owned());
        }
    }

    Some((exit_ip?.to_string(), country))
}
