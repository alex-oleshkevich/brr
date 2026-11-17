pub mod files;

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use brr_core::{Error, InstanceId, Size};
use brr_proto::Spawned;
use brr_proto::frames::{Link, Stream};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{OnceCell, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use self::files::{MAX_TRANSFER_BYTES, PUT_DONE, Request};
use crate::common::{EVENTS_QUEUE, die_with_parent, internal, millis};
use crate::{DownloadState, DriverEvent, LaunchSpec};

#[cfg(test)]
mod tests;

pub const DOCKER: &str = "docker";
pub const RUNNER_SUBCOMMAND: &str = "runner";
// The runner's HTTP proxy inside the container's own network namespace; the browser's only way out.
pub const RELAY_HOST: &str = "127.0.0.1";
pub const RELAY_PORT: u16 = 3128;
pub const CONTAINER_PREFIX: &str = "brr-";
pub const OWNER_LABEL: &str = "brr.owner";
// "run" for what one run leaves on a Host, "build" for the folder caches runs share.
pub const KIND_LABEL: &str = "brr.kind";
const RUN_KIND: &str = "run";
const BUILD_KIND: &str = "build";
// A remote run's files live in this volume, named after the Instance.
pub const RUN_VOLUME_PREFIX: &str = "brr-run-";

const SANDBOX_FIELD: &str = "sandbox";
// The rootfs is read-only; the browser's HOME and scratch files live here.
const TMPFS: &str = "/tmp:rw,nosuid,size=256m";
const CONTAINER_HOME: &str = "HOME=/tmp";
const DOCKER_HELP: &str =
    "install Docker and let the daemon's user run `docker`, or set sandbox.mode to none";
// `--rm` removes a container whose runner exited; these mean it is gone or about to be.
const ALREADY_GONE: [&str; 2] = ["No such container", "is already in progress"];
const MAX_ERROR_CHARS: usize = 500;
// The image's own system dirs; never mounted from the host.
const IMAGE_DIRS: [&str; 6] = ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/etc"];
// Characters that end or quote a field of `--mount`'s comma-separated syntax.
const MOUNT_SEPARATORS: [char; 2] = [',', '"'];
// Under the run dir: the copies of the extensions a sandboxed Chromium loads.
const EXTENSIONS_DIR: &str = "extensions";
// A browser build or asset folder copied to a Host once, named by the hash of its path.
const BUILD_VOLUME_PREFIX: &str = "brr-build-";
// Folders of the run volume, mounted at the Profile, download and run dir paths.
const PROFILE_SUBPATH: &str = "profile";
const DOWNLOADS_SUBPATH: &str = "downloads";
const RUN_SUBPATH: &str = "run";
const RUN_VOLUME_MODE: &str = "0700";
// Where a helper container sees the volume it fills.
const HELPER_MOUNT: &str = "/v";
const TUNNEL_QUEUE: usize = 64;
const MAX_ANSWER_BYTES: u64 = 4096;
// Next to the Profile folder: the copy coming back from a Host, and the one it replaces.
const INCOMING_SUFFIX: &str = ".incoming";
const PREVIOUS_SUFFIX: &str = ".previous";
// Host preparation runs before the browser's start timeout and has bounds of its own: a 1.4 GB
// image or a browser folder over SSH takes minutes on a slow link.
const IMAGE_COPY_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const SYNC_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const RUN_VOLUME_TIMEOUT: Duration = Duration::from_secs(2 * 60);

// Images, build folders and run volumes each Host already has, for the daemon's lifetime. One
// cell per key, so launches racing onto a new Host copy the image once.
static ON_HOST: LazyLock<Mutex<HashMap<String, Arc<OnceCell<()>>>>> = LazyLock::new(Mutex::default);

// What the daemon decides per launch; the engine turns it into a `docker run`.
#[derive(Debug, Clone)]
pub struct SandboxSpec {
    // $XDG_DATA_HOME/brr/runs/<instance_id>; holds the extension copies.
    pub run_dir: PathBuf,
    pub docker: DockerTarget,
    // A persistent Profile: a remote run sends the Profile folder back once the browser exited.
    pub copy_back: bool,
}

#[derive(Debug, Clone)]
pub struct DockerTarget {
    pub image: String,
    // The brr binary inside the image.
    pub runner: PathBuf,
    // Paths under a root are mounted from it; paths under none are expected inside the image.
    pub roots: Vec<MountRoot>,
    // "uid:gid" of the daemon, so files the browser writes stay the daemon's.
    pub user: String,
    pub memory_mb: u32,
    pub cpus: f64,
    pub pids: u32,
    pub shm_mb: u32,
    // OWNER_LABEL value: identifies this daemon's containers for the orphan sweep.
    pub owner: String,
    // None: brr's own Docker. Some: a remote Host such as ssh://user@host, where nothing of brr's
    // filesystem exists, so the run's files travel over the container's stdio.
    pub docker_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MountRoot {
    // The path as the daemon sees it; mounted at the same path in the container.
    pub path: PathBuf,
    pub source: MountSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MountSource {
    // A host path the daemon can name directly (daemon not in a container, or a bind mount).
    Bind(PathBuf),
    // A named Docker volume mounted at `MountRoot::path` in the daemon's own container.
    Volume(String),
}

// Sent to `brr runner` on the run's first stream. Holds no secret.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunnerSpec {
    // Must equal the runner's CARGO_PKG_VERSION; a mismatched image fails with a clear error.
    pub version: String,
    pub instance_id: InstanceId,
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    // Some for Display::Virtual: the runner starts Xvfb and sets DISPLAY/XAUTHORITY.
    pub xvfb: Option<Size>,
    // The folders mounted read-write; puts and gets reach nothing else.
    pub writable: Vec<PathBuf>,
    // A remote run: the browser starts only once brr sent Start after its puts.
    pub wait_for_files: bool,
    // The Profile folder to send back once the browser exited.
    pub copy_back: Option<PathBuf>,
}

// A remote run's file traffic after the start: downloads and the Profile copy-back.
pub(crate) struct Remote {
    link: Link,
    download_dir: PathBuf,
    instance_id: InstanceId,
    // Routes the runner's streams; ends with the connection, after the copy-back.
    routed: JoinHandle<()>,
}

fn container_name(instance_id: &InstanceId) -> String {
    format!("{CONTAINER_PREFIX}{instance_id}")
}

fn run_volume(instance_id: &InstanceId) -> String {
    format!("{RUN_VOLUME_PREFIX}{instance_id}")
}

fn build_volume(dir: &Path) -> String {
    let digest = Sha256::digest(dir.as_os_str().as_encoded_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();

    format!("{BUILD_VOLUME_PREFIX}{hex}")
}

fn missing_docker() -> Error {
    Error::DependencyMissing {
        name: DOCKER.to_owned(),
        help: DOCKER_HELP.to_owned(),
    }
}

fn not_started(error: io::Error) -> Error {
    match error.kind() {
        ErrorKind::NotFound => missing_docker(),
        _ => internal(error),
    }
}

// Docker's and the runner's messages may quote the Host's output: no control characters, capped.
fn printable(text: &[u8]) -> String {
    String::from_utf8_lossy(text)
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_ERROR_CHARS)
        .collect()
}

fn failed(args: &[&str], stderr: &[u8]) -> Error {
    internal(format!(
        "`docker {}` failed: {}",
        args.join(" "),
        printable(stderr)
    ))
}

fn utf8(text: &OsStr) -> Result<String, Error> {
    text.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::InvalidArgument {
            field: SANDBOX_FIELD.to_owned(),
            message: "the docker sandbox passes only UTF-8 paths, arguments and environment; rename the path or change the setting that is not UTF-8".to_owned(),
        })
}

fn mount_field(path: &Path) -> Result<String, Error> {
    let text = utf8(path.as_os_str())?;
    if text.contains(MOUNT_SEPARATORS) {
        return Err(Error::InvalidArgument {
            field: SANDBOX_FIELD.to_owned(),
            message: format!(
                "{text} cannot be mounted into the docker sandbox because it contains ',' or '\"'; rename it"
            ),
        });
    }

    Ok(text)
}

// The `--mount` values that put each path at the same path in the container; a path wanted both
// read-write and read-only is mounted read-write.
pub(crate) fn mounts(roots: &[MountRoot], paths: &[(&Path, bool)]) -> Result<Vec<String>, Error> {
    let mut wanted: BTreeMap<&Path, bool> = BTreeMap::new();
    for &(path, readonly) in paths {
        let readonly = wanted.get(path).map_or(readonly, |seen| *seen && readonly);
        wanted.insert(path, readonly);
    }

    let mut mounts = Vec::new();
    for (path, readonly) in wanted {
        if in_image(path) {
            continue;
        }
        let Some(root) = roots
            .iter()
            .filter(|root| path.starts_with(&root.path))
            .max_by_key(|root| root.path.components().count())
        else {
            continue;
        };
        let rest = path.strip_prefix(&root.path).map_err(internal)?;
        let dst = mount_field(path)?;

        let mut mount = match &root.source {
            MountSource::Bind(source) if rest.as_os_str().is_empty() => {
                format!("type=bind,src={},dst={dst}", mount_field(source)?)
            }
            MountSource::Bind(source) => {
                format!(
                    "type=bind,src={},dst={dst}",
                    mount_field(&source.join(rest))?
                )
            }
            MountSource::Volume(name) if rest.as_os_str().is_empty() => {
                format!(
                    "type=volume,src={},dst={dst}",
                    mount_field(Path::new(name))?
                )
            }
            MountSource::Volume(name) => format!(
                "type=volume,src={},dst={dst},volume-subpath={}",
                mount_field(Path::new(name))?,
                mount_field(rest)?
            ),
        };
        if readonly {
            mount.push_str(",readonly");
        }
        mounts.push(mount);
    }

    Ok(mounts)
}

// The image brings its own system: a host /usr/bin over it breaks every program inside.
fn in_image(path: &Path) -> bool {
    IMAGE_DIRS.iter().any(|dir| path.starts_with(dir))
}

// The read-only folders a launch needs from brr's machine: the browser's own folder, and
// Camoufox's fonts and add-ons. spec.extensions are copies inside the read-write run dir
// (copy_extensions): a read-only mount of their own would block Chromium's ruleset index again.
// Firefox keeps an add-on's rulesets in the profile, so Camoufox add-ons stay read-only.
fn read_only_dirs(spec: &LaunchSpec) -> Vec<&Path> {
    let mut dirs: Vec<&Path> = spec.executable.parent().into_iter().collect();
    if let Some(camoufox) = &spec.camoufox {
        dirs.push(camoufox.fonts_dir.as_path());
        dirs.extend(camoufox.addons.iter().map(PathBuf::as_path));
    }

    dirs
}

// On a Host the run volume's folders sit at the paths brr uses, so argv and CDP paths stay as
// they are; read-only folders come from build volumes `sync_dir` filled.
fn remote_mounts(spec: &LaunchSpec, sandbox: &SandboxSpec) -> Result<Vec<String>, Error> {
    let volume = run_volume(&spec.instance_id);
    let mut mounts = Vec::new();
    for (dir, subpath) in [
        (&spec.user_data_dir, PROFILE_SUBPATH),
        (&spec.download_dir, DOWNLOADS_SUBPATH),
        (&sandbox.run_dir, RUN_SUBPATH),
    ] {
        mounts.push(format!(
            "type=volume,src={volume},dst={},volume-subpath={subpath}",
            mount_field(dir)?
        ));
    }
    for dir in read_only_dirs(spec) {
        if !in_image(dir) {
            mounts.push(format!(
                "type=volume,src={},dst={},readonly",
                build_volume(dir),
                mount_field(dir)?
            ));
        }
    }

    Ok(mounts)
}

// Chromium writes an unpacked extension's indexed declarativeNetRequest rulesets into the
// extension's own folder, and a container must not change the library other Profiles load: a
// sandboxed launch loads copies in its run dir, which `command` mounts read-write.
pub(crate) fn copy_extensions(
    run_dir: &Path,
    extensions: &[PathBuf],
) -> Result<Vec<PathBuf>, Error> {
    extensions
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let copy = run_dir.join(EXTENSIONS_DIR).join(index.to_string());
            copy_dir(source, &copy).map_err(|error| {
                internal(format!(
                    "cannot copy the extension {} into the sandbox: {error}",
                    source.display()
                ))
            })?;

            Ok(copy)
        })
        .collect()
}

// The library holds only folders and regular files; following a link could copy a host file in.
fn copy_dir(source: &Path, dest: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest = dest.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &dest)?;
        } else {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("{} is neither a file nor a folder", entry.path().display()),
            ));
        }
    }

    Ok(())
}

// The browser command, built as for a launch without the sandbox, as the runner will start it.
pub(crate) fn runner_spec(
    spec: &LaunchSpec,
    sandbox: &SandboxSpec,
    browser: &Command,
    xvfb: Option<Size>,
) -> Result<RunnerSpec, Error> {
    let browser = browser.as_std();
    let remote = sandbox.docker.docker_url.is_some();

    let mut argv = vec![utf8(browser.get_program())?];
    for arg in browser.get_args() {
        argv.push(utf8(arg)?);
    }
    let mut env = BTreeMap::new();
    for (key, value) in browser.get_envs() {
        if let Some(value) = value {
            env.insert(utf8(key)?, utf8(value)?);
        }
    }

    Ok(RunnerSpec {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        instance_id: spec.instance_id,
        argv,
        env,
        xvfb,
        writable: vec![
            spec.user_data_dir.clone(),
            spec.download_dir.clone(),
            sandbox.run_dir.clone(),
        ],
        wait_for_files: remote,
        copy_back: (remote && sandbox.copy_back).then(|| spec.user_data_dir.clone()),
    })
}

// A `docker run` of `brr runner`, which starts the browser on fds 3 and 4 and relays them over
// the container's stdio.
pub(crate) fn command(spec: &LaunchSpec, sandbox: &SandboxSpec) -> Result<Command, Error> {
    let docker = &sandbox.docker;

    let writable = [&spec.user_data_dir, &spec.download_dir, &sandbox.run_dir];
    for dir in writable {
        std::fs::create_dir_all(dir).map_err(internal)?;
    }
    let mounts = match &docker.docker_url {
        Some(_) => remote_mounts(spec, sandbox)?,
        None => {
            let mut paths: Vec<(&Path, bool)> = writable
                .into_iter()
                .map(|dir| (dir.as_path(), false))
                .collect();
            paths.extend(read_only_dirs(spec).into_iter().map(|dir| (dir, true)));

            mounts(&docker.roots, &paths)?
        }
    };

    if let Some(dir) = spec.log_file.parent() {
        std::fs::create_dir_all(dir).map_err(internal)?;
    }
    // A file, never a pipe: an unread full pipe blocks the browser.
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&spec.log_file)
        .map_err(internal)?;

    let mut command = Command::new(DOCKER);
    if let Some(url) = &docker.docker_url {
        command.args(["-H", url]);
    }
    command
        .args(["run", "--rm", "-i", "--name"])
        .arg(container_name(&spec.instance_id))
        .arg("--label")
        .arg(format!("{OWNER_LABEL}={}", docker.owner))
        .args(["--network", "none", "--read-only", "--tmpfs", TMPFS])
        .arg("--shm-size")
        .arg(format!("{}m", docker.shm_mb))
        .args([
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--user",
            docker.user.as_str(),
        ])
        .arg("--memory")
        .arg(format!("{}m", docker.memory_mb))
        .arg("--memory-swap")
        .arg(format!("{}m", docker.memory_mb))
        .arg("--cpus")
        .arg(docker.cpus.to_string())
        .arg("--pids-limit")
        .arg(docker.pids.to_string())
        .args(["--env", CONTAINER_HOME]);
    for mount in mounts {
        command.arg("--mount").arg(mount);
    }
    command
        .arg("--entrypoint")
        .arg(&docker.runner)
        .arg(&docker.image)
        .arg(RUNNER_SUBCOMMAND)
        .stderr(log)
        .process_group(0)
        .kill_on_drop(false);
    die_with_parent(&mut command);

    Ok(command)
}

fn spawn(command: Command) -> Result<Spawned, Error> {
    brr_proto::spawn_stdio(command).map_err(|error| match error {
        brr_proto::Error::Io(error) => not_started(error),
        error => internal(error),
    })
}

// Starts the run's container and hands the runner its spec. On a remote Host the image, the
// read-only folders and the run volume are made ready first (at once when `prepare` did it), and
// the Profile and run folder are uploaded before the runner may start the browser.
pub(crate) async fn start(
    spec: &LaunchSpec,
    sandbox: &SandboxSpec,
    browser: &Command,
    xvfb: Option<Size>,
) -> Result<(Spawned, Option<Remote>), Error> {
    let runner_spec = runner_spec(spec, sandbox, browser, xvfb)?;
    let command = command(spec, sandbox)?;
    if let Some(url) = &sandbox.docker.docker_url {
        prepare_host(url, spec, sandbox).await?;
    }

    let mut spawned = spawn(command)?;
    let link = spawned
        .link
        .clone()
        .ok_or_else(|| internal("the sandbox runner has no framed connection"))?;
    if let Err(error) = hand_over(&link, spec, sandbox, &runner_spec).await {
        let _ = spawned.child.start_kill();

        return Err(error);
    }
    if sandbox.docker.docker_url.is_none() {
        return Ok((spawned, None));
    }

    let opened = spawned
        .opened
        .take()
        .ok_or_else(|| internal("the sandbox runner has no framed connection"))?;
    let (tunnels, tunnels_rx) = mpsc::channel(TUNNEL_QUEUE);
    spawned.opened = Some(tunnels_rx);
    let routed = tokio::spawn(route(
        opened,
        tunnels,
        runner_spec.copy_back,
        spec.instance_id,
    ));

    Ok((
        spawned,
        Some(Remote {
            link,
            download_dir: spec.download_dir.clone(),
            instance_id: spec.instance_id,
            routed,
        }),
    ))
}

async fn hand_over(
    link: &Link,
    spec: &LaunchSpec,
    sandbox: &SandboxSpec,
    runner_spec: &RunnerSpec,
) -> Result<(), Error> {
    let mut stream = open(link, &Request::Spec)?;
    let json = serde_json::to_vec(runner_spec).map_err(internal)?;
    stream.write_all(&json).await.map_err(internal)?;
    stream.shutdown().await.map_err(internal)?;
    if !runner_spec.wait_for_files {
        return Ok(());
    }

    for dir in [&spec.user_data_dir, &sandbox.run_dir] {
        put(link, dir).await?;
    }
    open(link, &Request::Start)?;

    Ok(())
}

fn open(link: &Link, request: &Request) -> Result<Stream, Error> {
    let request = serde_json::to_vec(request).map_err(internal)?;

    link.open(&request).map_err(internal)
}

// The folder goes to the same path on the Host; the runner answers once it is unpacked.
async fn put(link: &Link, dir: &Path) -> Result<(), Error> {
    let stream = open(link, &Request::Put(dir.to_owned()))?;
    let (mut answer, writer) = tokio::io::split(stream);

    let mut writer = files::pack(dir.to_owned(), writer).await.map_err(|error| {
        internal(format!(
            "cannot send {} to the Host: {error}",
            dir.display()
        ))
    })?;
    writer.shutdown().await.map_err(internal)?;

    let mut text = Vec::new();
    (&mut answer)
        .take(MAX_ANSWER_BYTES)
        .read_to_end(&mut text)
        .await
        .map_err(internal)?;
    if text != PUT_DONE.as_bytes() {
        return Err(internal(format!(
            "the Host did not take {}: {}",
            dir.display(),
            printable(&text)
        )));
    }

    Ok(())
}

async fn route(
    mut opened: mpsc::Receiver<Stream>,
    tunnels: mpsc::Sender<Stream>,
    copy_back: Option<PathBuf>,
    instance_id: InstanceId,
) {
    while let Some(stream) = opened.recv().await {
        if stream.request().is_empty() {
            let _ = tunnels.send(stream).await;
            continue;
        }

        match (serde_json::from_slice(stream.request()), &copy_back) {
            (Ok(Request::Profile), Some(dir)) => receive_profile(stream, dir, &instance_id).await,
            _ => warn!(%instance_id, "the runner opened a stream brr does not expect; refused"),
        }
    }
}

fn with_suffix(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = dir.as_os_str().to_owned();
    name.push(suffix);

    PathBuf::from(name)
}

fn remove_dir(dir: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

// The Profile changes only when the whole archive arrived: a dropped connection or a hostile
// archive leaves the copy from before the run.
async fn receive_profile(stream: Stream, dir: &Path, instance_id: &InstanceId) {
    let incoming = with_suffix(dir, INCOMING_SUFFIX);
    let unpacked = match remove_dir(&incoming) {
        Ok(()) => files::unpack(stream, incoming.clone()).await,
        Err(error) => Err(error),
    };

    let dir = dir.to_owned();
    let taken = tokio::task::spawn_blocking(move || match unpacked {
        Ok(()) => swap(&dir, &incoming),
        Err(error) => {
            let _ = remove_dir(&incoming);
            Err(error)
        }
    })
    .await
    .map_err(io::Error::other)
    .and_then(|taken| taken);

    match taken {
        Ok(()) => info!(%instance_id, "Profile copied back from the Host"),
        Err(error) => warn!(
            %instance_id,
            %error,
            "Profile not copied back from the Host; it keeps its copy from before the run"
        ),
    }
}

fn swap(dir: &Path, incoming: &Path) -> io::Result<()> {
    let previous = with_suffix(dir, PREVIOUS_SUFFIX);
    std::fs::set_permissions(incoming, std::fs::metadata(dir)?.permissions())?;
    remove_dir(&previous)?;

    std::fs::rename(dir, &previous)?;
    if let Err(error) = std::fs::rename(incoming, dir) {
        std::fs::rename(&previous, dir)?;
        return Err(error);
    }

    remove_dir(&previous)
}

impl Remote {
    // Completed downloads are fetched into the local download dir before the daemon hears of
    // them, and Exited waits for the Profile copy-back.
    pub(crate) fn relay(
        self,
        mut events: mpsc::Receiver<DriverEvent>,
    ) -> mpsc::Receiver<DriverEvent> {
        let (relayed, relayed_rx) = mpsc::channel(EVENTS_QUEUE);

        tokio::spawn(async move {
            let mut routed = Some(self.routed);
            while let Some(mut event) = events.recv().await {
                match &mut event {
                    DriverEvent::DownloadProgress {
                        guid,
                        received_bytes,
                        total_bytes,
                        state: state @ DownloadState::Completed,
                    } => match fetch(&self.link, &self.download_dir, guid).await {
                        Ok(size) => {
                            *received_bytes = size;
                            *total_bytes = Some(size);
                        }
                        Err(error) => {
                            warn!(instance_id = %self.instance_id, %error, "download not fetched from the Host");
                            *state = DownloadState::Canceled;
                        }
                    },
                    DriverEvent::Exited { .. } => {
                        if let Some(routed) = routed.take() {
                            let _ = routed.await;
                        }
                    }
                    _ => {}
                }
                if relayed.send(event).await.is_err() {
                    return;
                }
            }
        });

        relayed_rx
    }
}

// The browser names the file, so the name must be one plain path segment.
async fn fetch(link: &Link, download_dir: &Path, guid: &str) -> io::Result<u64> {
    let mut components = Path::new(guid).components();
    let path = match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) => download_dir.join(name),
        _ => {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "the browser named the download with a path",
            ));
        }
    };

    let request = serde_json::to_vec(&Request::Get(path.clone())).map_err(io::Error::other)?;
    let mut stream = link.open(&request)?;
    stream.shutdown().await?;
    let size = stream.read_u64().await?;
    if size > MAX_TRANSFER_BYTES {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "the download is over the {} GiB transfer cap",
                MAX_TRANSFER_BYTES >> 30
            ),
        ));
    }

    let mut file = tokio::fs::File::create(&path).await?;
    let copied = tokio::io::copy(&mut (&mut stream).take(size), &mut file).await?;
    if copied != size {
        drop(file);
        let _ = tokio::fs::remove_file(&path).await;

        return Err(io::Error::new(
            ErrorKind::UnexpectedEof,
            "the Host's connection ended inside the download",
        ));
    }

    Ok(size)
}

async fn once(key: String, work: impl Future<Output = Result<(), Error>>) -> Result<(), Error> {
    let cell = ON_HOST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(key)
        .or_default()
        .clone();

    cell.get_or_try_init(|| work).await.map(drop)
}

// The next launch on the Host checks its image and build volumes again.
pub(crate) fn forget_host(url: &str) {
    let prefix = format!("{url} ");

    ON_HOST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|key, _| !key.starts_with(&prefix));
}

// Only the executable's own folder is mounted: a symlink such as /usr/bin/google-chrome-stable
// must point into it.
pub(crate) fn resolve_executable(executable: &Path) -> Result<PathBuf, Error> {
    std::fs::canonicalize(executable).map_err(|error| Error::Internal {
        message: format!(
            "cannot resolve the browser executable {}: {error}",
            executable.display()
        ),
    })
}

// Makes a remote Host ready for the launch: image, browser and asset folders, run volume. The
// daemon calls it before the start timeout begins; the launch itself then finds it all done.
// Nothing to do for a launch without a Host.
pub async fn prepare(spec: &LaunchSpec) -> Result<(), Error> {
    let Some(sandbox) = &spec.sandbox else {
        return Ok(());
    };
    let Some(url) = &sandbox.docker.docker_url else {
        return Ok(());
    };
    let mut resolved = spec.clone();
    resolved.executable = resolve_executable(&spec.executable)?;

    let prepared = prepare_host(url, &resolved, sandbox).await;
    if prepared.is_err() {
        forget_host(url);
    }

    prepared
}

async fn bounded(
    step: &str,
    url: &str,
    limit: Duration,
    work: impl Future<Output = Result<(), Error>>,
) -> Result<(), Error> {
    let started = Instant::now();
    match tokio::time::timeout(limit, work).await {
        Ok(done) => {
            if done.is_ok() {
                debug!(
                    url,
                    step,
                    elapsed_ms = millis(started.elapsed()),
                    "Host step done"
                );
            }
            done
        }
        Err(_) => {
            warn!(url, step, limit_s = limit.as_secs(), "Host step timed out");
            Err(Error::Timeout {
                after_ms: millis(limit),
            })
        }
    }
}

async fn prepare_host(url: &str, spec: &LaunchSpec, sandbox: &SandboxSpec) -> Result<(), Error> {
    let docker_target = &sandbox.docker;
    let image = &docker_target.image;
    // By id, not by name: a local image rebuilt under the same tag must reach the Host again.
    let local_id = image_id(None, image).await?;
    once(
        format!("{url} {local_id}"),
        bounded(
            "image copy",
            url,
            IMAGE_COPY_TIMEOUT,
            copy_image(url, image, &local_id),
        ),
    )
    .await?;

    for dir in read_only_dirs(spec) {
        if in_image(dir) {
            continue;
        }
        let volume = build_volume(dir);
        once(
            format!("{url} {volume}"),
            bounded(
                "folder sync",
                url,
                SYNC_TIMEOUT,
                sync_dir(url, docker_target, dir, &volume),
            ),
        )
        .await?;
    }

    once(
        run_volume_key(url, &spec.instance_id),
        bounded(
            "run volume",
            url,
            RUN_VOLUME_TIMEOUT,
            create_run_volume(url, &spec.instance_id, docker_target),
        ),
    )
    .await
}

fn run_volume_key(url: &str, instance_id: &InstanceId) -> String {
    format!("{url} {}", run_volume(instance_id))
}

// A new volume gets these labels from the first container that mounts it.
fn helper_mount(volume: &str, kind: &str, owner: &str) -> Result<String, Error> {
    Ok(format!(
        "type=volume,src={volume},dst={HELPER_MOUNT},volume-label={OWNER_LABEL}={},volume-label={KIND_LABEL}={kind}",
        mount_field(Path::new(owner))?
    ))
}

async fn image_id(url: Option<&str>, image: &str) -> Result<String, Error> {
    let id = docker(url, &["image", "inspect", "--format", "{{.Id}}", image]).await?;

    Ok(id.trim().to_owned())
}

// A Host without the image, or with another image under its name, gets brr's copy.
async fn copy_image(url: &str, image: &str, local_id: &str) -> Result<(), Error> {
    if image_id(Some(url), image)
        .await
        .is_ok_and(|host_id| host_id == local_id)
    {
        return Ok(());
    }
    info!(url, image, "copying the sandbox image to the Host");
    let started = Instant::now();

    let mut save = Command::new(DOCKER)
        .args(["save", image])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(not_started)?;
    let saved: Stdio = save
        .stdout
        .take()
        .ok_or_else(|| internal("`docker save` has no stdout"))?
        .try_into()
        .map_err(internal)?;
    let load = Command::new(DOCKER)
        .args(["-H", url, "load"])
        .stdin(saved)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();

    let (saved, loaded) = tokio::join!(save.wait_with_output(), load);
    let saved = saved.map_err(internal)?;
    if !saved.status.success() {
        return Err(failed(&["save", image], &saved.stderr));
    }
    let loaded = loaded.map_err(not_started)?;
    if !loaded.status.success() {
        return Err(failed(&["-H", url, "load"], &loaded.stderr));
    }

    info!(
        url,
        image,
        elapsed_s = started.elapsed().as_secs(),
        "sandbox image copied to the Host"
    );

    Ok(())
}

// A folder from brr's machine, copied once into a volume of its own on the Host; the archive's
// marker shows the copy finished.
async fn sync_dir(
    url: &str,
    docker_target: &DockerTarget,
    dir: &Path,
    volume: &str,
) -> Result<(), Error> {
    let image = docker_target.image.as_str();
    let mount = helper_mount(volume, BUILD_KIND, &docker_target.owner)?;
    let marker = format!("{HELPER_MOUNT}/{}", files::COMPLETE_MARKER);
    let check = [
        "run",
        "--rm",
        "--network",
        "none",
        "--mount",
        &mount,
        "--entrypoint",
        "test",
        image,
        "-e",
        &marker,
    ];
    if docker(Some(url), &check).await.is_ok() {
        return Ok(());
    }
    info!(url, dir = %dir.display(), "copying a browser folder to the Host");
    let started = Instant::now();

    // As root, so the files keep their owner: the daemon's uid, which the run's --user names.
    let args = [
        "-H",
        url,
        "run",
        "--rm",
        "-i",
        "--network",
        "none",
        "--user",
        "0:0",
        "--mount",
        &mount,
        "--entrypoint",
        "tar",
        image,
        "-x",
        "-C",
        HELPER_MOUNT,
    ];
    let mut child = Command::new(DOCKER)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(not_started)?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| internal("`docker run` has no stdin"))?;
    // Dropping stdin ends the archive for tar.
    let packed = files::pack(dir.to_owned(), stdin).await.map(drop);
    let output = child.wait_with_output().await.map_err(internal)?;

    packed.map_err(|error| {
        internal(format!(
            "cannot copy {} to the Host: {error}",
            dir.display()
        ))
    })?;
    if !output.status.success() {
        return Err(failed(&args, &output.stderr));
    }

    info!(
        url,
        dir = %dir.display(),
        elapsed_s = started.elapsed().as_secs(),
        "browser folder copied to the Host"
    );

    Ok(())
}

// Docker mounts a volume-subpath only when it exists, and a new volume belongs to root: a helper
// container makes the run's folders and gives them to the run's user.
async fn create_run_volume(
    url: &str,
    instance_id: &InstanceId,
    docker_target: &DockerTarget,
) -> Result<(), Error> {
    let (uid, gid) = docker_target
        .user
        .split_once(':')
        .ok_or_else(|| internal("the sandbox user is not uid:gid"))?;
    let mount = helper_mount(&run_volume(instance_id), RUN_KIND, &docker_target.owner)?;
    let dirs = [PROFILE_SUBPATH, DOWNLOADS_SUBPATH, RUN_SUBPATH]
        .map(|dir| format!("{HELPER_MOUNT}/{dir}"));

    let mut args = vec![
        "run",
        "--rm",
        "--network",
        "none",
        "--user",
        "0:0",
        "--mount",
        &mount,
        "--entrypoint",
        "install",
        &docker_target.image,
        "-d",
        "-m",
        RUN_VOLUME_MODE,
        "-o",
        uid,
        "-g",
        gid,
    ];
    args.extend(dirs.iter().map(String::as_str));

    docker(Some(url), &args).await.map(drop)
}

// `url` is a remote Docker such as ssh://user@host; None is the local one.
pub async fn docker(url: Option<&str>, args: &[&str]) -> Result<String, Error> {
    let mut command = Command::new(DOCKER);
    if let Some(url) = url {
        command.args(["-H", url]);
    }
    let output = command
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(not_started)?;
    if !output.status.success() {
        return Err(failed(args, &output.stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

// A remote run's volume goes with its container.
pub async fn remove_container(url: Option<&str>, instance_id: &InstanceId) {
    let name = container_name(instance_id);

    if let Err(error) = docker(url, &["rm", "-f", &name]).await
        && !already_gone(&error)
    {
        warn!(%error, container = %name, "sandbox container not removed");
    }

    if let Some(url) = url {
        let volume = run_volume(instance_id);
        if let Err(error) = docker(Some(url), &["volume", "rm", "-f", &volume]).await {
            warn!(%error, %volume, "sandbox run volume not removed");
        }
        ON_HOST
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&run_volume_key(url, instance_id));
    }
}

fn already_gone(error: &Error) -> bool {
    let message = error.to_string();

    ALREADY_GONE.iter().any(|text| message.contains(text))
}

// Containers and run volumes a previous run of this daemon left behind when it died; build
// volumes are caches and stay.
pub async fn sweep(url: Option<&str>, owner: &str) -> Result<usize, Error> {
    let filter = format!("label={OWNER_LABEL}={owner}");
    let listed = docker(url, &["ps", "-aq", "--filter", &filter]).await?;
    let ids: Vec<&str> = listed.split_whitespace().collect();
    if !ids.is_empty() {
        let mut args = vec!["rm", "-f"];
        args.extend(&ids);
        if let Err(error) = docker(url, &args).await
            && !already_gone(&error)
        {
            return Err(error);
        }
    }

    if url.is_some() {
        let kind = format!("label={KIND_LABEL}={RUN_KIND}");
        removed_volumes(url, &filter, &kind).await?;
    }

    Ok(ids.len())
}

async fn removed_volumes(url: Option<&str>, owner: &str, kind: &str) -> Result<usize, Error> {
    let listed = docker(
        url,
        &["volume", "ls", "-q", "--filter", owner, "--filter", kind],
    )
    .await?;
    let volumes: Vec<&str> = listed.split_whitespace().collect();
    if !volumes.is_empty() {
        let mut args = vec!["volume", "rm", "-f"];
        args.extend(&volumes);
        docker(url, &args).await?;
    }

    Ok(volumes.len())
}

// For a Host that is deleted: its build volumes of this daemon go, and the next Host at the same
// URL starts from nothing. Returns how many volumes were removed.
pub async fn forget_host_storage(url: &str, owner: &str) -> Result<usize, Error> {
    forget_host(url);
    let owner = format!("label={OWNER_LABEL}={owner}");
    let kind = format!("label={KIND_LABEL}={BUILD_KIND}");

    removed_volumes(Some(url), &owner, &kind).await
}
