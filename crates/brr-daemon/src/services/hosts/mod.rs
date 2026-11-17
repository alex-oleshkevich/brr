use std::{
    collections::{HashMap, HashSet},
    future::Future,
    ops::RangeInclusive,
    process::Stdio,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use brr_core::{ConflictReason, Error, Host, HostId, HostStatus, now_ms};
use brr_engine::sandbox;
use brr_store::host;
use futures::future;
use tokio::{process::Command, sync::Mutex as AsyncMutex, time};
use tracing::{info, warn};

use crate::{
    app::App,
    services::{host::parse_meminfo, profiles::validate_name, sandbox as sandbox_service},
};

#[cfg(test)]
mod tests;

const SSH: &str = "ssh";
const SSH_SCHEME: &str = "ssh://";
const MEMINFO_COMMAND: [&str; 2] = ["cat", "/proc/meminfo"];
const DOCKER_VERSION: [&str; 3] = ["version", "--format", "{{.Server.Version}}"];
const NAME_FIELD: &str = "name";
const DOCKER_URL_FIELD: &str = "docker_url";
const MAX_INSTANCES_FIELD: &str = "max_instances";
pub const MAX_INSTANCES: RangeInclusive<u32> = 1..=64;
// Each step crosses SSH to another machine; `docker -H ssh://… version` was measured at 1.4 s.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
// A launch is placed on a status at most this old; older ones are probed again first.
const PLACEMENT_MAX_AGE: Duration = Duration::from_secs(30);
const MAX_ERROR_CHARS: usize = 300;
const KB_PER_MB: u64 = 1024;
const SSH_HELP: &str =
    "check that the daemon's user can run `ssh <user@host> docker version` without a prompt";

#[derive(Default)]
pub struct Hosts {
    // The last probe of each Host; empty after a restart until something checks.
    probed: Mutex<HashMap<HostId, HostStatus>>,
    // One refresh at a time: a burst of launches must not open a burst of SSH connections.
    refreshing: AsyncMutex<()>,
    // Hosts whose startup sweep and probe are still running: a container started there now would
    // be swept with the orphans.
    sweeping: Mutex<HashSet<HostId>>,
}

pub struct NewHost {
    pub name: String,
    pub docker_url: String,
    pub max_instances: u32,
}

pub struct HostChanges {
    pub name: Option<String>,
    pub docker_url: Option<String>,
    pub max_instances: Option<u32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SshTarget<'a> {
    // user@host
    pub destination: &'a str,
    pub port: Option<u16>,
}

impl Hosts {
    // The stored Host with the last probe; `running` stays the store's live count.
    fn with_status(&self, mut found: Host) -> Host {
        let probed = self.probed.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(status) = probed.get(&found.id) {
            found.status = HostStatus {
                running: found.status.running,
                ..status.clone()
            };
        }

        found
    }

    fn record(&self, host_id: HostId, status: HostStatus) {
        self.probed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(host_id, status);
    }

    fn forget(&self, host_id: &HostId) {
        self.probed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(host_id);
    }

    // A launch that failed there: placement skips the Host until its next probe.
    pub fn launch_failed(&self, host_id: HostId, message: String) {
        let mut probed = self.probed.lock().unwrap_or_else(PoisonError::into_inner);
        let status = probed.entry(host_id).or_default();
        status.reachable = false;
        status.error = Some(sanitize(&message));
        status.checked_at = Some(now_ms());
    }

    fn sweeping(&self) -> HashSet<HostId> {
        self.sweeping
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_sweeping(&self, host_id: HostId, sweeping: bool) {
        let mut swept = self.sweeping.lock().unwrap_or_else(PoisonError::into_inner);
        if sweeping {
            swept.insert(host_id);
        } else {
            swept.remove(&host_id);
        }
    }
}

// Only ssh://user@host[:port]: the URL reaches `docker -H` and `ssh`, so nothing in it may read
// as an option or carry a path, query or second host.
pub fn parse_docker_url(url: &str) -> Result<SshTarget<'_>, Error> {
    let invalid = || Error::InvalidArgument {
        field: DOCKER_URL_FIELD.to_owned(),
        message: format!(
            "{url:?} must look like ssh://user@host or ssh://user@host:port, using A-Z, a-z, 0-9, `.`, `_`, `-`"
        ),
    };

    let rest = url.strip_prefix(SSH_SCHEME).ok_or_else(invalid)?;
    let (destination, port) = match rest.rsplit_once(':') {
        Some((destination, port)) => {
            let port: u16 = port.parse().map_err(|_| invalid())?;
            if port == 0 {
                return Err(invalid());
            }

            (destination, Some(port))
        }
        None => (rest, None),
    };
    let (user, hostname) = destination.split_once('@').ok_or_else(invalid)?;
    let plain = |part: &str| {
        !part.is_empty()
            && !part.starts_with('-')
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    };
    if !plain(user) || !plain(hostname) {
        return Err(invalid());
    }

    Ok(SshTarget { destination, port })
}

// Browser counts for the capacity display; `local_*` keeps the meaning it had before Hosts.
#[derive(Debug, PartialEq, Eq)]
pub struct Capacity {
    pub local_running: u32,
    // On every Host, usable now or not.
    pub hosts_running: u32,
    // Of the Hosts placement may use now: enabled and reachable.
    pub hosts_max: u32,
    // No room anywhere: local and every usable Host are at their max.
    pub at_capacity: bool,
}

// `placed`: admitted runs per place (None: local); `hosts`: the Hosts placement may consider.
pub fn capacity(placed: &HashMap<Option<HostId>, u32>, local_max: u32, hosts: &[Host]) -> Capacity {
    let on = |place: Option<HostId>| placed.get(&place).copied().unwrap_or(0);
    let usable: Vec<&Host> = hosts
        .iter()
        .filter(|found| found.enabled && found.status.reachable)
        .collect();
    let local_running = on(None);

    Capacity {
        local_running,
        hosts_running: placed
            .iter()
            .filter(|(place, _)| place.is_some())
            .map(|(_, running)| running)
            .sum(),
        hosts_max: usable.iter().map(|found| found.max_instances).sum(),
        at_capacity: local_running >= local_max
            && usable
                .iter()
                .all(|found| on(Some(found.id)) >= found.max_instances),
    }
}

fn validate(name: &str, docker_url: &str, max_instances: u32) -> Result<(), Error> {
    validate_name(name, NAME_FIELD)?;
    parse_docker_url(docker_url)?;
    if !MAX_INSTANCES.contains(&max_instances) {
        return Err(Error::InvalidArgument {
            field: MAX_INSTANCES_FIELD.to_owned(),
            message: format!(
                "must be {} to {}",
                MAX_INSTANCES.start(),
                MAX_INSTANCES.end()
            ),
        });
    }

    Ok(())
}

pub async fn list(app: &App) -> Result<Vec<Host>, Error> {
    let hosts = app.store.run(|c| host::all(c)).await?;

    Ok(hosts
        .into_iter()
        .map(|found| app.hosts.with_status(found))
        .collect())
}

pub async fn get(app: &App, id_or_name: String) -> Result<Host, Error> {
    let found = app.store.run(move |c| host::find(c, &id_or_name)).await?;

    Ok(app.hosts.with_status(found))
}

pub async fn create(app: &App, new: NewHost) -> Result<Host, Error> {
    validate(&new.name, &new.docker_url, new.max_instances)?;

    let created = Host {
        id: HostId::new(),
        name: new.name,
        docker_url: new.docker_url,
        max_instances: new.max_instances,
        enabled: true,
        created_at: now_ms(),
        status: HostStatus::default(),
    };
    let stored = created.clone();
    app.store.run(move |c| host::insert(c, &stored)).await?;
    info!(host_id = %created.id, name = %created.name, docker_url = %created.docker_url, "host added");

    check_host(app, created).await
}

pub async fn update(app: &App, id_or_name: String, changes: HostChanges) -> Result<Host, Error> {
    let mut found = app.store.run(move |c| host::find(c, &id_or_name)).await?;
    found.name = changes.name.unwrap_or(found.name);
    found.docker_url = changes.docker_url.unwrap_or(found.docker_url);
    found.max_instances = changes.max_instances.unwrap_or(found.max_instances);
    found.enabled = changes.enabled.unwrap_or(found.enabled);
    validate(&found.name, &found.docker_url, found.max_instances)?;

    let stored = found.clone();
    app.store.run(move |c| host::update(c, &stored)).await?;

    check_host(app, found).await
}

// A Host with browsers on it keeps its row: their runs name it.
pub async fn delete(app: &App, id_or_name: String) -> Result<Host, Error> {
    let deleted = app
        .store
        .run(move |c| {
            let found = host::find(c, &id_or_name)?;
            let running = found.status.running;
            if running > 0 {
                return Err(Error::Conflict {
                    reason: ConflictReason::Referenced,
                    detail: Some(format!(
                        "{running} browsers run on host {}; stop them first",
                        found.name
                    )),
                });
            }
            host::delete(c, &found.id)?;

            Ok(found)
        })
        .await?;
    app.hosts.forget(&deleted.id);
    info!(host_id = %deleted.id, name = %deleted.name, "host deleted");

    // The row is gone either way; the Host's build volumes of this daemon are best effort.
    let owner = sandbox_service::owner(&app.paths);
    match bounded(sandbox::forget_host_storage(&deleted.docker_url, &owner)).await {
        Ok(count) => info!(name = %deleted.name, count, "host build volumes removed"),
        Err(message) => warn!(name = %deleted.name, %message, "host build volumes not removed"),
    }

    Ok(deleted)
}

pub async fn check(app: &App, id_or_name: String) -> Result<Host, Error> {
    let found = app.store.run(move |c| host::find(c, &id_or_name)).await?;

    check_host(app, found).await
}

// Enabled Hosts a launch may go to, each with a status no older than PLACEMENT_MAX_AGE.
pub async fn for_placement(app: &App) -> Result<Vec<Host>, Error> {
    let _refreshing = app.hosts.refreshing.lock().await;
    let now = now_ms();
    let checks = cached(app).await?.into_iter().map(|found| async move {
        if fresh(found.status.checked_at, now) {
            Ok(found)
        } else {
            check_host(app, found).await
        }
    });

    future::join_all(checks).await.into_iter().collect()
}

// Enabled Hosts placement may use, with their last status, probing none.
pub async fn cached(app: &App) -> Result<Vec<Host>, Error> {
    let sweeping = app.hosts.sweeping();

    Ok(list(app)
        .await?
        .into_iter()
        .filter(|found| found.enabled && !sweeping.contains(&found.id))
        .collect())
}

fn fresh(checked_at: Option<i64>, now: i64) -> bool {
    let max_age_ms = i64::try_from(PLACEMENT_MAX_AGE.as_millis()).unwrap_or(i64::MAX);

    checked_at.is_some_and(|at| (0..max_age_ms).contains(&now.saturating_sub(at)))
}

pub async fn docker_url(app: &App, host_id: HostId) -> Result<String, Error> {
    let found = app.store.run(move |c| host::get(c, &host_id)).await?;

    Ok(found.docker_url)
}

// Each enabled Host is swept of what a daemon that died left there, then probed, in the
// background. Placement skips a Host until both are done, so no new run is swept and the first
// launch finds a known status; a failed sweep leaves its orphans, never a new run.
pub async fn sweep_all(app: &Arc<App>) {
    let found = match app.store.run(|c| host::all(c)).await {
        Ok(found) => found,
        Err(error) => {
            warn!(%error, "hosts not read; their orphan containers stay");

            return;
        }
    };
    let owner = sandbox_service::owner(&app.paths);

    for found in found.into_iter().filter(|found| found.enabled) {
        app.hosts.set_sweeping(found.id, true);
        // Weak: a strong reference would keep the daemon lock while SSH hangs on the sweep.
        let app = Arc::downgrade(app);
        let owner = owner.clone();
        tokio::spawn(async move {
            match bounded(sandbox::sweep(Some(&found.docker_url), &owner)).await {
                Ok(count) => info!(host = %found.name, count, "orphan sandbox containers removed"),
                Err(message) => {
                    warn!(host = %found.name, %message, "orphan sandbox containers not removed");
                }
            }
            let Some(app) = app.upgrade() else {
                return;
            };
            let (host_id, name) = (found.id, found.name.clone());
            if let Err(error) = check_host(&app, found).await {
                warn!(host = %name, %error, "host not checked at startup");
            }
            app.hosts.set_sweeping(host_id, false);
        });
    }
}

async fn check_host(app: &App, mut found: Host) -> Result<Host, Error> {
    let status = probe(app, &found.docker_url, found.status.running).await;
    info!(
        host_id = %found.id,
        reachable = status.reachable,
        error = status.error.as_deref(),
        "host checked"
    );
    app.hosts.record(found.id, status.clone());
    found.status = status;

    Ok(found)
}

async fn probe(app: &App, docker_url: &str, running: u32) -> HostStatus {
    let mut status = HostStatus {
        running,
        checked_at: Some(now_ms()),
        ..HostStatus::default()
    };

    if let Err(message) = bounded(sandbox::docker(Some(docker_url), &DOCKER_VERSION)).await {
        status.error = Some(format!(
            "docker on {docker_url} did not answer: {message}; {SSH_HELP}"
        ));

        return status;
    }
    status.reachable = true;

    let (memory, image_present) = tokio::join!(memory(docker_url), image_present(app, docker_url));
    match memory {
        Ok((total_mb, available_mb)) => {
            status.mem_total_mb = Some(total_mb);
            status.mem_available_mb = Some(available_mb);
        }
        Err(message) => status.error = Some(message),
    }
    status.image_present = image_present;

    status
}

async fn memory(docker_url: &str) -> Result<(u64, u64), String> {
    let target = parse_docker_url(docker_url).map_err(|error| error.to_string())?;
    let mut command = Command::new(SSH);
    command.args(["-o", "BatchMode=yes"]);
    if let Some(port) = target.port {
        command.arg("-p").arg(port.to_string());
    }
    command
        .arg("--")
        .arg(target.destination)
        .args(MEMINFO_COMMAND)
        .stdin(Stdio::null())
        .kill_on_drop(true);

    let output = match time::timeout(PROBE_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(format!("ssh could not run ({error})")),
        Err(_) => return Err(timed_out(SSH)),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);

        return Err(format!(
            "memory not read over ssh: {}; {SSH_HELP}",
            sanitize(stderr.trim())
        ));
    }

    let (total_kb, available_kb) = parse_meminfo(&String::from_utf8_lossy(&output.stdout))
        .ok_or_else(|| "the host's /proc/meminfo has no MemTotal or MemAvailable".to_owned())?;

    Ok((total_kb / KB_PER_MB, available_kb / KB_PER_MB))
}

// None when the daemon has no sandbox image to look for (sandbox.image unset outside a container).
async fn image_present(app: &App, docker_url: &str) -> Option<bool> {
    let image = app.sandbox.target(app).await.ok()?.image;
    let inspected = bounded(sandbox::docker(
        Some(docker_url),
        &["image", "inspect", &image],
    ))
    .await;

    Some(inspected.is_ok())
}

async fn bounded<T>(call: impl Future<Output = Result<T, Error>>) -> Result<T, String> {
    match time::timeout(PROBE_TIMEOUT, call).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(Error::Internal { message })) => Err(sanitize(&message)),
        Ok(Err(error)) => Err(sanitize(&error.to_string())),
        Err(_) => Err(timed_out(sandbox::DOCKER)),
    }
}

fn timed_out(program: &str) -> String {
    format!(
        "{program} did not answer within {} s; {SSH_HELP}",
        PROBE_TIMEOUT.as_secs()
    )
}

// stderr of ssh and docker on another machine is untrusted text.
fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_ERROR_CHARS)
        .collect()
}
