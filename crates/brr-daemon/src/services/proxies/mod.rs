use std::{
    collections::HashMap,
    net::Ipv6Addr,
    sync::{Arc, Mutex, PoisonError, atomic::Ordering},
    time::Duration,
};

use brr_core::{
    ConflictReason, Error, EventData, InstanceId, Page, PoolId, PoolMode, Profile, Proxy,
    ProxyChoice, ProxyId, ProxyScheme, Secret, Traffic, now_ms,
};
use brr_netproxy::{
    Forwarder, Listen, Meter, Route as NetRoute, Upstream, check as check_upstream,
};
use brr_proto::frames::Stream;
use brr_store::{Connection, instance, pool, profile, proxy};
use futures::{StreamExt, stream};
use tokio::time;
use tracing::{info, warn};

use crate::{
    app::App,
    ops::invalid,
    services::secrets::{Secrets, status_of},
};

#[cfg(test)]
mod tests;

pub const STALE_AFTER_MS: i64 = 3_600_000;
pub const CHECK_CONCURRENCY: usize = 8;

const HOST_MAX_CHARS: usize = 253;
const USERNAME_MAX_CHARS: usize = 255;
const PASSWORD_MAX_CHARS: usize = 1024;
const IMPORT_MAX_PARTS: usize = 4;
const SECS_PER_MIN: u64 = 60;

// The running forwarder of one launch, with the upstream and the Proxy it was picked from; a
// sandboxed launch without a Proxy has a direct forwarder and neither.
struct Route {
    upstream: Option<Arc<Upstream>>,
    forwarder: Forwarder,
    proxy_id: Option<ProxyId>,
    meter: Arc<Meter>,
}

#[derive(Default)]
pub struct ProxyTable {
    forwarders: Mutex<HashMap<InstanceId, Route>>,
    rotation: Mutex<HashMap<PoolId, usize>>,
}

impl ProxyTable {
    fn next_rotation(&self, pool_id: &PoolId) -> usize {
        let mut rotation = self.rotation.lock().unwrap_or_else(PoisonError::into_inner);
        let counter = rotation.entry(*pool_id).or_default();
        let current = *counter;
        *counter += 1;

        current
    }
}

pub struct NewProxy {
    pub scheme: ProxyScheme,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<Secret>,
}

pub struct ProxyChanges {
    pub scheme: Option<ProxyScheme>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<Secret>,
    pub clear_password: bool,
}

pub struct Imported {
    pub ids: Vec<ProxyId>,
    pub duplicates: u32,
    pub invalid_lines: Vec<u32>,
}

fn aad(proxy_id: &ProxyId) -> String {
    format!("proxy:{proxy_id}")
}

fn validate_host(host: &str) -> Result<(), Error> {
    let literal = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .is_some_and(|inner| inner.parse::<Ipv6Addr>().is_ok());
    let name = (1..=HOST_MAX_CHARS).contains(&host.len())
        && host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-'));

    if literal || name {
        return Ok(());
    }

    Err(invalid(
        "host",
        "use a host name of up to 253 letters, digits, `.` and `-`, or a bracketed IPv6 address",
    ))
}

fn validate(
    host: &str,
    port: u16,
    username: Option<&str>,
    password: Option<&Secret>,
) -> Result<(), Error> {
    validate_host(host)?;
    if port == 0 {
        return Err(invalid("port", "use a port from 1 to 65535"));
    }
    if username.is_some_and(|u| u.chars().count() > USERNAME_MAX_CHARS || u.contains(':')) {
        return Err(invalid("username", "use at most 255 characters and no `:`"));
    }
    if password.is_some_and(|p| p.expose().chars().count() > PASSWORD_MAX_CHARS) {
        return Err(invalid("password", "use at most 1024 characters"));
    }

    Ok(())
}

pub fn parse_import(scheme: ProxyScheme, text: &str) -> (Vec<NewProxy>, Vec<u32>) {
    let mut proxies = Vec::new();
    let mut invalid_lines = Vec::new();

    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match parse_line(scheme, line) {
            Some(parsed) => proxies.push(parsed),
            None => invalid_lines.push(u32::try_from(index + 1).unwrap_or(u32::MAX)),
        }
    }

    (proxies, invalid_lines)
}

// Two forms: `[scheme://][user[:password]@]host:port` (a scheme here wins over the import's) and
// `host:port[:user:password]`. The password may contain `:`, so the colon form stops at four parts.
fn parse_line(scheme: ProxyScheme, line: &str) -> Option<NewProxy> {
    let (scheme, host, port, username, password) = match line.split_once("://") {
        Some((named, rest)) => {
            let scheme = match named {
                "http" => ProxyScheme::Http,
                "https" => ProxyScheme::Https,
                "socks5" => ProxyScheme::Socks5,
                _ => return None,
            };
            let (login, address) = rest.rsplit_once('@').unwrap_or(("", rest));
            let (host, port) = address.rsplit_once(':')?;
            let (username, password) = login.split_once(':').unwrap_or((login, ""));
            (scheme, host, port, username, password)
        }
        None if has_login_address(line) => {
            let (login, address) = line.rsplit_once('@')?;
            let (host, port) = address.rsplit_once(':')?;
            let (username, password) = login.split_once(':').unwrap_or((login, ""));
            (scheme, host, port, username, password)
        }
        None => {
            let mut parts = line.splitn(IMPORT_MAX_PARTS, ':');
            let host = parts.next()?;
            let port = parts.next()?;
            let (username, password) = match (parts.next(), parts.next()) {
                (None, _) => ("", ""),
                (Some(username), Some(password)) => (username, password),
                (Some(_), None) => return None,
            };
            (scheme, host, port, username, password)
        }
    };
    let port: u16 = port.parse().ok()?;
    let username = Some(username.to_string()).filter(|u| !u.is_empty());
    let password = Some(Secret::new(password)).filter(|p| !p.expose().is_empty());
    validate(host, port, username.as_deref(), password.as_ref()).ok()?;

    Some(NewProxy {
        scheme,
        host: host.to_string(),
        port,
        username,
        password,
    })
}

// `login@host:port` ends in a numeric port after its last `@`; in the colon form an `@` is part of
// the password (`host:port:user:p@ss`) and what follows it is no address.
fn has_login_address(line: &str) -> bool {
    line.rsplit_once('@')
        .and_then(|(_, address)| address.rsplit_once(':'))
        .is_some_and(|(_, port)| port.parse::<u16>().is_ok())
}

fn read_password(
    c: &Connection,
    secrets: &Secrets,
    proxy_id: &ProxyId,
) -> Result<Option<Secret>, Error> {
    let Some(sealed) = proxy::password_enc(c, proxy_id)? else {
        return Ok(None);
    };

    secrets.open(&aad(proxy_id), &sealed).map(Some)
}

fn with_status(c: &Connection, secrets: &Secrets, mut found: Proxy) -> Result<Proxy, Error> {
    found.password = read_password(c, secrets, &found.id)?.map(|p| status_of(&p));

    Ok(found)
}

pub async fn load(app: &Arc<App>, proxy_id: &ProxyId) -> Result<Proxy, Error> {
    let app_ = Arc::clone(app);
    let proxy_id = *proxy_id;

    app.store
        .run(move |c| with_status(c, &app_.secrets, proxy::get(c, &proxy_id)?))
        .await
}

pub async fn list(app: &Arc<App>, page: Page) -> Result<(Vec<Proxy>, Option<String>), Error> {
    let app_ = Arc::clone(app);

    app.store
        .run(move |c| {
            let (rows, next) = proxy::list(c, &page)?;
            let rows = rows
                .into_iter()
                .map(|found| with_status(c, &app_.secrets, found))
                .collect::<Result<Vec<_>, Error>>()?;

            Ok((rows, next))
        })
        .await
}

// A duplicate is named in the message, so a bulk import can count it and a single create can say which.
fn ensure_unused(c: &Connection, candidate: &Proxy, except: Option<&ProxyId>) -> Result<(), Error> {
    let taken = proxy::find_endpoint(c, candidate)?.is_some_and(|id| Some(&id) != except);
    if !taken {
        return Ok(());
    }

    Err(Error::Conflict {
        reason: ConflictReason::NameTaken,
        detail: Some(format!(
            "proxy {}:{} already exists",
            candidate.host, candidate.port
        )),
    })
}

pub async fn create(app: &Arc<App>, new: NewProxy) -> Result<Proxy, Error> {
    let id = insert(app, new).await?;
    spawn_checks(app, vec![id]);

    load(app, &id).await
}

pub async fn update(
    app: &Arc<App>,
    proxy_id: &ProxyId,
    changes: ProxyChanges,
) -> Result<Proxy, Error> {
    if changes.password.is_some() && changes.clear_password {
        return Err(invalid(
            "password",
            "give a password or clear_password, not both",
        ));
    }
    let app_ = Arc::clone(app);
    let proxy_id = *proxy_id;

    app.store
        .run(move |c| {
            let mut found = proxy::get(c, &proxy_id)?;
            found.scheme = changes.scheme.unwrap_or(found.scheme);
            found.host = changes.host.unwrap_or(found.host);
            found.port = changes.port.unwrap_or(found.port);
            found.username = match changes.username {
                Some(username) if username.is_empty() => None,
                Some(username) => Some(username),
                None => found.username,
            };
            validate(
                &found.host,
                found.port,
                found.username.as_deref(),
                changes.password.as_ref(),
            )?;
            ensure_unused(c, &found, Some(&proxy_id))?;

            proxy::update(c, &found)?;
            if let Some(password) = &changes.password {
                let sealed = app_.secrets.seal(&aad(&proxy_id), password)?;
                proxy::set_password(c, &proxy_id, Some(&sealed))?;
            } else if changes.clear_password {
                proxy::set_password(c, &proxy_id, None)?;
            }

            proxy::clear_health(c, &proxy_id)
        })
        .await?;
    spawn_checks(app, vec![proxy_id]);

    load(app, &proxy_id).await
}

pub async fn delete(app: &Arc<App>, proxy_id: &ProxyId) -> Result<Proxy, Error> {
    let proxy_id = *proxy_id;

    app.store
        .run(move |c| {
            let found = proxy::get(c, &proxy_id)?;
            let users = profile::using_proxy(c, &proxy_id)?;
            if !users.is_empty() {
                let names: Vec<String> = users.into_iter().map(|(_, name)| name).collect();

                return Err(Error::Conflict {
                    reason: ConflictReason::Referenced,
                    detail: Some(format!("profile {}", names.join(", "))),
                });
            }
            proxy::delete(c, &proxy_id)?;

            Ok(found)
        })
        .await
}

pub async fn import(app: &Arc<App>, scheme: ProxyScheme, text: &str) -> Result<Imported, Error> {
    let (parsed, invalid_lines) = parse_import(scheme, text);
    let mut ids = Vec::new();
    let mut duplicates = 0;

    for new in parsed {
        match insert(app, new).await {
            Ok(id) => ids.push(id),
            Err(Error::Conflict {
                reason: ConflictReason::NameTaken,
                ..
            }) => duplicates += 1,
            Err(error) => return Err(error),
        }
    }
    spawn_checks(app, ids.clone());

    Ok(Imported {
        ids,
        duplicates,
        invalid_lines,
    })
}

// An import checks all its rows together, so insert does not start a check.
async fn insert(app: &Arc<App>, new: NewProxy) -> Result<ProxyId, Error> {
    validate(
        &new.host,
        new.port,
        new.username.as_deref(),
        new.password.as_ref(),
    )?;
    let id = ProxyId::new();
    let sealed = new
        .password
        .as_ref()
        .map(|p| app.secrets.seal(&aad(&id), p))
        .transpose()?;
    let row = Proxy {
        id,
        scheme: new.scheme,
        host: new.host,
        port: new.port,
        username: new.username,
        password: None,
        healthy: None,
        exit_ip: None,
        country: None,
        latency_ms: None,
        last_error: None,
        checked_at: None,
    };
    app.store
        .run(move |c| {
            ensure_unused(c, &row, None)?;

            proxy::insert(c, &row, sealed.as_deref())
        })
        .await?;

    Ok(id)
}

pub async fn upstream(app: &Arc<App>, proxy_id: &ProxyId) -> Result<Upstream, Error> {
    let app_ = Arc::clone(app);
    let proxy_id = *proxy_id;

    app.store
        .run(move |c| {
            let found = proxy::get(c, &proxy_id)?;

            Ok(Upstream {
                scheme: found.scheme,
                host: found.host,
                port: found.port,
                username: found.username,
                password: read_password(c, &app_.secrets, &proxy_id)?,
            })
        })
        .await
}

pub async fn check(app: &Arc<App>, proxy_id: &ProxyId) -> Result<Proxy, Error> {
    let upstream = Arc::new(upstream(app, proxy_id).await?);
    let check_url = app.settings().proxy.check_url.clone();
    // Nothing is held across the check: it can take as long as CHECK_TIMEOUT.
    let result = check_upstream(upstream, &check_url).await;
    let health = match result {
        Ok(reached) => proxy::Health {
            healthy: true,
            exit_ip: Some(reached.exit_ip),
            country: reached.country,
            latency_ms: Some(reached.latency_ms),
            last_error: None,
            checked_at: now_ms(),
        },
        Err(reason) => proxy::Health {
            healthy: false,
            exit_ip: None,
            country: None,
            latency_ms: None,
            last_error: Some(reason),
            checked_at: now_ms(),
        },
    };
    let event = EventData::ProxyHealth {
        proxy_id: *proxy_id,
        healthy: health.healthy,
        exit_ip: health.exit_ip.clone(),
        country: health.country.clone(),
        latency_ms: health.latency_ms,
    };
    info!(%proxy_id, healthy = health.healthy, latency_ms = ?health.latency_ms, "proxy checked");

    let id = *proxy_id;
    app.store
        .run(move |c| proxy::set_health(c, &id, &health))
        .await?;
    app.events.publish(event).await?;

    load(app, proxy_id).await
}

async fn check_all(app: &Arc<App>, ids: Vec<ProxyId>) {
    stream::iter(ids)
        .for_each_concurrent(CHECK_CONCURRENCY, |id| async move {
            if let Err(error) = check(app, &id).await {
                warn!(proxy_id = %id, error = format!("{error:#}"), "proxy check not recorded");
            }
        })
        .await;
}

fn spawn_checks(app: &Arc<App>, ids: Vec<ProxyId>) {
    let app = Arc::clone(app);

    tokio::spawn(async move { check_all(&app, ids).await });
}

pub fn spawn_health(app: Arc<App>) {
    // Held weakly: a strong reference would keep the daemon lock after the last owner drops the App.
    let app = Arc::downgrade(&app);

    tokio::spawn(async move {
        loop {
            let Some(strong) = app.upgrade() else {
                return;
            };
            let minutes = u64::from(strong.settings().proxy.health_interval_min).max(1);
            drop(strong);
            time::sleep(Duration::from_secs(minutes * SECS_PER_MIN)).await;

            let Some(strong) = app.upgrade() else {
                return;
            };
            let ids = strong.store.run(|c| proxy::all(c)).await;
            match ids {
                Ok(rows) => check_all(&strong, rows.into_iter().map(|p| p.id).collect()).await,
                Err(error) => warn!(
                    error = format!("{error:#}"),
                    "listing proxies for the health round failed"
                ),
            }
        }
    });
}

pub fn ensure_exists(c: &Connection, choice: Option<&ProxyChoice>) -> Result<(), Error> {
    let missing = |what: String| Error::InvalidArgument {
        field: "settings.proxy".to_string(),
        message: format!("{what} not found"),
    };

    match choice {
        None => Ok(()),
        Some(ProxyChoice::ProxyId(id)) => {
            proxy::get(c, id).map(drop).map_err(|error| match error {
                Error::NotFound { .. } => missing(format!("proxy {id}")),
                other => other,
            })
        }
        Some(ProxyChoice::PoolId(id)) => pool::get(c, id).map(drop).map_err(|error| match error {
            Error::NotFound { .. } => missing(format!("pool {id}")),
            other => other,
        }),
    }
}

async fn refreshed(app: &Arc<App>, proxy_id: &ProxyId) -> Result<Proxy, Error> {
    let found = load(app, proxy_id).await?;
    let stale = found
        .checked_at
        .is_none_or(|at| now_ms() - at > STALE_AFTER_MS);

    if stale {
        return check(app, proxy_id).await;
    }

    Ok(found)
}

async fn pick(app: &Arc<App>, profile: &Profile, choice: &ProxyChoice) -> Result<ProxyId, Error> {
    let pool_id = match choice {
        ProxyChoice::ProxyId(proxy_id) => {
            let found = refreshed(app, proxy_id).await?;

            return if found.healthy == Some(true) {
                Ok(*proxy_id)
            } else {
                Err(Error::ProxyUnhealthy)
            };
        }
        ProxyChoice::PoolId(pool_id) => *pool_id,
    };

    let profile_id = profile.id;
    let (members, sticky) = app
        .store
        .run(move |c| {
            Ok((
                pool::get(c, &pool_id)?,
                pool::sticky(c, &pool_id, &profile_id)?,
            ))
        })
        .await?;
    if members.mode == PoolMode::Sticky
        && let Some(held) = sticky
        && members.proxy_ids.contains(&held)
        && refreshed(app, &held)
            .await
            .is_ok_and(|found| found.healthy == Some(true))
    {
        return Ok(held);
    }

    // Owned ids: a borrowed item here trips higher-ranked lifetime inference in the spawned launch.
    let checked: Vec<_> = stream::iter(members.proxy_ids.clone())
        .map(|id| async move { refreshed(app, &id).await })
        .buffered(CHECK_CONCURRENCY)
        .collect()
        .await;
    let mut healthy: Vec<ProxyId> = checked
        .into_iter()
        .filter_map(|result| match result {
            Ok(found) => (found.healthy == Some(true)).then_some(found.id),
            Err(error) => {
                warn!(%pool_id, error = format!("{error:#}"), "pool member not checked");

                None
            }
        })
        .collect();
    healthy.sort();
    let Some(first) = healthy.first().copied() else {
        return Err(Error::ProxyUnhealthy);
    };

    match members.mode {
        PoolMode::Rotate => Ok(healthy[app.proxies.next_rotation(&pool_id) % healthy.len()]),
        PoolMode::Sticky => {
            app.store
                .run(move |c| pool::set_sticky(c, &pool_id, &profile_id, &first))
                .await?;

            Ok(first)
        }
    }
}

// A sandboxed launch always gets a forwarder, its browser's only way out: it listens nowhere and
// serves the tunnels the runner opens over the container's stdio (`serve_tunnel`).
pub async fn for_launch(
    app: &Arc<App>,
    instance_id: &InstanceId,
    profile: &Profile,
    sandboxed: bool,
) -> Result<Option<String>, Error> {
    let proxy_id = match &profile.settings.proxy {
        Some(choice) => Some(pick(app, profile, choice).await?),
        None if !sandboxed => return Ok(None),
        None => None,
    };
    let upstream = match &proxy_id {
        Some(proxy_id) => Some(Arc::new(upstream(app, proxy_id).await?)),
        None => None,
    };

    let route = match &upstream {
        Some(upstream) => NetRoute::Upstream(Arc::clone(upstream)),
        None => NetRoute::Direct,
    };
    let listen = if sandboxed {
        Listen::None
    } else {
        Listen::Loopback
    };
    let meter = Arc::new(Meter::default());
    let forwarder = Forwarder::start(route, listen, Arc::clone(&meter)).await?;
    let url = forwarder.url();
    app.proxies
        .forwarders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            *instance_id,
            Route {
                upstream,
                forwarder,
                proxy_id,
                meter,
            },
        );

    Ok(url)
}

// A run without a forwarder (it ended) just closes the tunnel.
pub fn serve_tunnel(app: &Arc<App>, instance_id: &InstanceId, tunnel: Stream) {
    if let Some(route) = app
        .proxies
        .forwarders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(instance_id)
    {
        route.forwarder.serve(tunnel);
    }
}

pub fn release(app: &Arc<App>, instance_id: &InstanceId) {
    app.proxies
        .forwarders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(instance_id);
}

// What a run's forwarder has counted so far; None for a run without one.
pub fn live_traffic(app: &Arc<App>, instance_id: &InstanceId) -> Option<Traffic> {
    app.proxies
        .forwarders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(instance_id)
        .map(|route| Traffic {
            sent_bytes: route.meter.sent.load(Ordering::Relaxed),
            received_bytes: route.meter.received.load(Ordering::Relaxed),
        })
}

// A run that ended: its totals go to the record before the forwarder is dropped.
pub async fn finish(app: &Arc<App>, instance_id: &InstanceId) {
    let totals = live_traffic(app, instance_id);
    release(app, instance_id);
    let Some(totals) = totals else {
        return;
    };

    let id = *instance_id;
    if let Err(error) = app
        .store
        .run(move |c| instance::set_traffic(c, &id, &totals))
        .await
    {
        warn!(instance_id = %instance_id, %error, "run traffic not recorded");
    }
}

// The proxy the launch picked; a pool's pick differs between launches.
pub fn instance_proxy_id(app: &Arc<App>, instance_id: &InstanceId) -> Option<ProxyId> {
    app.proxies
        .forwarders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(instance_id)
        .and_then(|route| route.proxy_id)
}

// The exit IP the launch's proxy showed at its last health check.
pub async fn exit_ip(app: &Arc<App>, instance_id: &InstanceId) -> Result<Option<String>, Error> {
    let Some(proxy_id) = instance_proxy_id(app, instance_id) else {
        return Ok(None);
    };

    Ok(load(app, &proxy_id).await?.exit_ip)
}

pub fn instance_upstream(app: &Arc<App>, instance_id: &InstanceId) -> Option<Arc<Upstream>> {
    app.proxies
        .forwarders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(instance_id)
        .and_then(|route| route.upstream.clone())
}
