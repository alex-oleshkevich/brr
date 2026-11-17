use std::{
    collections::HashMap,
    env, fs,
    io::ErrorKind,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex as StdMutex, PoisonError},
    time::Duration,
};

use brr_core::{Error, InstanceId, Paths, SandboxMode, Settings};
use brr_engine::sandbox::{CONTAINER_PREFIX, DOCKER, DockerTarget, MountRoot, MountSource};
use nix::unistd::{getegid, geteuid};
use serde::Deserialize;
use tokio::{
    process::Command,
    sync::{Mutex, OnceCell},
    time::{self, Instant},
};
use tracing::{debug, info, warn};

use crate::{app::App, internal};

#[cfg(test)]
mod tests;

// Where the brr image keeps its binary; the runner of a host daemon comes from that image.
const HOST_RUNNER: &str = "/usr/local/bin/brr";
const HOST_ROOT: &str = "/";
const HOSTNAME_ENV: &str = "HOSTNAME";
const HOSTNAME_FILE: &str = "/etc/hostname";
const VOLUME_MOUNT: &str = "volume";
const BIND_MOUNT: &str = "bind";
const DOCKER_TIMEOUT: Duration = Duration::from_secs(15);
const STATS_MAX_AGE: Duration = Duration::from_secs(10);
const MESSAGE_MAX_CHARS: usize = 300;
const IMAGE_SETTING: &str = "sandbox.image";
const DOCKER_FIX: &str =
    "mount /var/run/docker.sock and add brr's user to its group, or install the docker CLI";
// Longer suffixes first: every unit ends in B.
const MEM_UNITS: [(&str, f64); 10] = [
    ("KiB", 1024.0),
    ("MiB", 1_048_576.0),
    ("GiB", 1_073_741_824.0),
    ("TiB", 1_099_511_627_776.0),
    ("kB", 1e3),
    ("KB", 1e3),
    ("MB", 1e6),
    ("GB", 1e9),
    ("TB", 1e12),
    ("B", 1.0),
];

#[derive(Debug, Clone, PartialEq)]
pub struct Discovery {
    pub image: Option<String>,
    pub roots: Vec<MountRoot>,
    pub runner: PathBuf,
}

// The last `docker stats` of one Docker host, by Instance.
type Stats = Arc<Mutex<Option<(Instant, HashMap<InstanceId, u64>)>>>;

#[derive(Default)]
pub struct Sandbox {
    discovery: OnceCell<Discovery>,
    // Per Docker host (None: brr's own, else a Host's URL), one `docker stats` at a time; callers
    // within STATS_MAX_AGE share its result. Keyed by the configured Hosts, so it stays small.
    stats: StdMutex<HashMap<Option<String>, Stats>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Inspected {
    config: InspectedConfig,
    #[serde(default)]
    mounts: Vec<InspectedMount>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectedConfig {
    #[serde(default)]
    image: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InspectedMount {
    #[serde(rename = "Type")]
    kind: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    source: PathBuf,
    destination: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Stat {
    name: String,
    mem_usage: String,
}

impl Sandbox {
    // brr's own Docker; launch sets `docker_url` once admission placed the run on a Host.
    pub async fn target(&self, app: &App) -> Result<DockerTarget, Error> {
        let discovery = self.discovery.get_or_try_init(discover).await?;
        let settings = app.settings();
        let sandbox = &settings.sandbox;

        Ok(DockerTarget {
            image: image(discovery.image.as_deref(), &settings)?,
            runner: discovery.runner.clone(),
            roots: discovery.roots.clone(),
            user: format!("{}:{}", geteuid(), getegid()),
            memory_mb: sandbox.memory_mb,
            cpus: sandbox.cpus,
            pids: sandbox.pids,
            shm_mb: sandbox.shm_mb,
            owner: owner(&app.paths),
            docker_url: None,
        })
    }

    // The resident memory of every brr container on one Docker host, by Instance.
    pub async fn rss(&self, docker_url: Option<&str>) -> HashMap<InstanceId, u64> {
        let stats = Arc::clone(
            self.stats
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(docker_url.map(str::to_owned))
                .or_default(),
        );
        let mut cached = stats.lock().await;
        if let Some((read_at, rss)) = cached.as_ref()
            && read_at.elapsed() < STATS_MAX_AGE
        {
            return rss.clone();
        }

        let args = ["stats", "--no-stream", "--format", "{{json .}}"];
        let rss = match docker(docker_url, &args).await {
            Ok(output) => parse_stats(&output),
            Err(message) => {
                warn!(?docker_url, %message, "container memory not read");

                HashMap::new()
            }
        };
        *cached = Some((Instant::now(), rss.clone()));

        rss
    }
}

pub async fn discover() -> Result<Discovery, Error> {
    // On the host every path the browser needs is a host path: each one is bind-mounted as itself,
    // so system browsers outside the data dir (e.g. /opt/google/chrome) work too.
    if !brr_engine::in_container() {
        return Ok(Discovery {
            image: None,
            roots: vec![MountRoot {
                path: PathBuf::from(HOST_ROOT),
                source: MountSource::Bind(PathBuf::from(HOST_ROOT)),
            }],
            runner: PathBuf::from(HOST_RUNNER),
        });
    }

    // `docker inspect` accepts the container's hostname: its short id, or its name under uncloud.
    let hostname = match env::var(HOSTNAME_ENV) {
        Ok(name) if !name.is_empty() => name,
        _ => fs::read_to_string(HOSTNAME_FILE)
            .map_err(internal)?
            .trim()
            .to_owned(),
    };
    let inspected = docker(None, &["inspect", hostname.as_str()])
        .await
        .map_err(|message| Error::DependencyMissing {
            name: "docker".to_owned(),
            help: format!("{message}; {DOCKER_FIX}"),
        })?;
    let (image, roots) = parse_inspect(&inspected)?;

    Ok(Discovery {
        image,
        roots,
        runner: env::current_exe().map_err(internal)?,
    })
}

fn parse_inspect(json: &[u8]) -> Result<(Option<String>, Vec<MountRoot>), Error> {
    let inspected: Vec<Inspected> = serde_json::from_slice(json).map_err(internal)?;
    let Some(own) = inspected.into_iter().next() else {
        return Err(internal(
            "docker inspect found no container for this daemon",
        ));
    };

    let roots = own
        .mounts
        .into_iter()
        .filter_map(|mount| {
            let source = match mount.kind.as_str() {
                VOLUME_MOUNT => MountSource::Volume(mount.name?),
                BIND_MOUNT => MountSource::Bind(mount.source),
                _ => return None,
            };

            Some(MountRoot {
                path: mount.destination,
                source,
            })
        })
        .collect();
    let image = Some(own.config.image).filter(|image| !image.is_empty());

    Ok((image, roots))
}

// The setting wins over the daemon's own image.
pub fn image(discovered: Option<&str>, settings: &Settings) -> Result<String, Error> {
    settings
        .sandbox
        .image
        .as_deref()
        .or(discovered)
        .map(str::to_owned)
        .ok_or_else(|| Error::DependencyMissing {
            name: "sandbox image".to_owned(),
            help: format!("set {IMAGE_SETTING} to a brr image of the same version"),
        })
}

pub(crate) fn owner(paths: &Paths) -> String {
    paths.data_dir.to_string_lossy().into_owned()
}

pub async fn probe() -> Result<(), String> {
    docker(None, &["version", "--format", "{{.Server.Version}}"])
        .await
        .map(drop)
        .map_err(|message| format!("{message}; {DOCKER_FIX}"))
}

pub async fn image_present(image: &str) -> Result<(), String> {
    docker(None, &["image", "inspect", image])
        .await
        .map(drop)
        .map_err(|message| {
            format!(
                "{message}; pull {image} or set {IMAGE_SETTING} to an image this Docker host has"
            )
        })
}

// Under the daemon lock nothing runs yet: every run dir and every container of this data dir is
// left from a daemon that died.
pub async fn clear_leftovers(app: &App) {
    match fs::remove_dir_all(app.paths.runs_dir()) {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            warn!(%error, "leftover run dirs not removed");
        }
        _ => {}
    }

    if let Err(message) = probe().await {
        if app.settings().sandbox.mode == SandboxMode::Docker {
            warn!(%message, "docker unreachable; sandboxed launches will fail");
        } else {
            debug!(%message, "docker unreachable; orphan sweep skipped");
        }

        return;
    }

    match brr_engine::sandbox::sweep(None, &owner(&app.paths)).await {
        Ok(count) => info!(count, "orphan sandbox containers removed"),
        Err(error) => warn!(%error, "orphan sandbox containers not removed"),
    }
}

pub fn remove_run_dir(paths: &Paths, instance_id: &InstanceId) {
    let dir = paths.runs_dir().join(instance_id.to_string());
    match fs::remove_dir_all(&dir) {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            warn!(%instance_id, %error, dir = %dir.display(), "run dir not removed");
        }
        _ => {}
    }
}

// Docker's output is untrusted text: control characters go and the length is capped. `url` is a
// Host's Docker; None is brr's own.
async fn docker(url: Option<&str>, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new(DOCKER);
    if let Some(url) = url {
        command.args(["-H", url]);
    }
    let run = command
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = match time::timeout(DOCKER_TIMEOUT, run).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) if error.kind() == ErrorKind::NotFound => {
            return Err("the docker CLI is not installed".to_owned());
        }
        Ok(Err(error)) => return Err(format!("docker could not run ({error})")),
        Err(_) => {
            return Err(format!(
                "docker did not answer within {} s",
                DOCKER_TIMEOUT.as_secs()
            ));
        }
    };
    if output.status.success() {
        return Ok(output.stdout);
    }

    let stderr: String = String::from_utf8_lossy(&output.stderr)
        .chars()
        .filter(|c| !c.is_control())
        .take(MESSAGE_MAX_CHARS)
        .collect();

    Err(format!(
        "docker {} failed: {}",
        args.join(" "),
        stderr.trim()
    ))
}

fn parse_stats(output: &[u8]) -> HashMap<InstanceId, u64> {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| {
            let stat: Stat = serde_json::from_str(line).ok()?;
            let instance_id: InstanceId = stat.name.strip_prefix(CONTAINER_PREFIX)?.parse().ok()?;

            Some((instance_id, parse_mem_usage(&stat.mem_usage)?))
        })
        .collect()
}

// `512MiB / 2GiB`: the part before the slash is in use, the rest is the limit.
fn parse_mem_usage(text: &str) -> Option<u64> {
    let used = text.split('/').next()?.trim();
    let (number, factor) = MEM_UNITS
        .iter()
        .find_map(|(unit, factor)| Some((used.strip_suffix(unit)?, *factor)))?;
    let bytes = number.trim().parse::<f64>().ok()? * factor;

    // `as` saturates, and docker never reports negative usage.
    Some(bytes as u64)
}
