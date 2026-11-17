use std::io;
use std::net::Ipv4Addr;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use brr_engine::Xvfb;
use brr_engine::sandbox::files::{self, PUT_DONE, Request};
use brr_engine::sandbox::{RELAY_PORT, RUNNER_SUBCOMMAND, RunnerSpec};
use brr_proto::frames::{Link, Side, Stream};
use clap::Command;
use command_fds::{CommandFdExt, FdMapping};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixStream};
use tokio::process::{Child, Command as Process};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Semaphore, mpsc, oneshot};

#[cfg(test)]
mod tests;

pub const NAME: &str = RUNNER_SUBCOMMAND;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const IMAGE_HELP: &str = "set sandbox.image to an image of the same brr version";
const COMMANDS_FD: i32 = 3;
const MESSAGES_FD: i32 = 4;
const MAX_RELAYS: usize = 512;
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
const BUFFER_BYTES: usize = 64 * 1024;
const HOME: &str = "/tmp";
const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const XVFB_LOG: &str = "/tmp/xvfb.log";
const ENV_HOME: &str = "HOME";
const ENV_PATH: &str = "PATH";
const ENV_DISPLAY: &str = "DISPLAY";
const ENV_XAUTHORITY: &str = "XAUTHORITY";
const SIGNAL_EXIT_BASE: i32 = 128;
const FAILURE: i32 = 1;
// The spec carries Camoufox's config, hundreds of KB.
const MAX_SPEC_BYTES: u64 = 16 * 1024 * 1024;
// How long the browser gets to flush its profile once asked to stop or once it closed its pipe;
// below the daemon's default instance.stop_timeout_s (10 s), after which it removes the container.
const STOP_GRACE: Duration = Duration::from_secs(8);

// What ended the relay.
enum Ending {
    Exited(io::Result<ExitStatus>),
    // The browser closed its message pipe. Both browsers do that early in their shutdown after
    // Browser.close, before their profile is on disk.
    Closed,
    // brr dropped the command stream or the container was signaled.
    Stop,
}

pub fn command() -> Command {
    Command::new(NAME)
        .about("Run one browser inside a brr sandbox container")
        .hide(true)
}

fn parse(json: &[u8], version: &str) -> anyhow::Result<RunnerSpec> {
    let spec: RunnerSpec =
        serde_json::from_slice(json).context("the runner spec is not valid JSON")?;
    if spec.version != version {
        bail!(
            "runner version {version} does not match brr {}; {IMAGE_HELP}",
            spec.version
        );
    }

    Ok(spec)
}

// stdout carries only frames; everything else goes to stderr. The process exits from here
// because a pending blocking stdin read would otherwise hold the runtime open at shutdown.
pub async fn run() -> anyhow::Result<()> {
    let (link, pipe, mut accepted) =
        Link::start(tokio::io::stdin(), tokio::io::stdout(), Side::Runner);
    let spec = receive_spec(&mut accepted).await?;
    let (program, args) = spec
        .argv
        .split_first()
        .context("the runner spec has an empty argv")?;

    let (started, start) = oneshot::channel();
    tokio::spawn(serve_files(accepted, spec.writable.clone(), started));
    if spec.wait_for_files {
        start
            .await
            .context("brr closed the connection before the browser's files arrived")?;
    }

    let mut xvfb = match spec.xvfb.clone() {
        Some(size) => Some(Xvfb::start(size, &spec.instance_id, Path::new(XVFB_LOG)).await?),
        None => None,
    };

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, RELAY_PORT))
        .await
        .with_context(|| format!("cannot listen on 127.0.0.1:{RELAY_PORT} for the proxy relay"))?;
    let relays = Arc::new(Semaphore::new(MAX_RELAYS));
    let relay = link.clone();
    tokio::spawn(async move {
        loop {
            let Ok(permit) = relays.clone().acquire_owned().await else {
                return;
            };
            let mut client = match listener.accept().await {
                Ok((client, _)) => client,
                Err(error) => {
                    tracing::warn!(%error, "cannot accept a proxy connection");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
            };
            let mut tunnel = match relay.open(&[]) {
                Ok(tunnel) => tunnel,
                Err(error) => {
                    tracing::warn!(%error, "cannot open a proxy tunnel to brr");
                    return;
                }
            };

            tokio::spawn(async move {
                let _permit = permit;
                let _ = tokio::io::copy_bidirectional(&mut client, &mut tunnel).await;
            });
        }
    });

    // Juggler shuts these fds down with shutdown(2), which only works on sockets.
    let (commands_read, commands_write) = StdUnixStream::pair()?;
    let (messages_read, messages_write) = StdUnixStream::pair()?;
    let output = std::io::stderr().as_fd().try_clone_to_owned()?;

    let mut command = Process::new(program);
    command
        .args(args)
        .env_clear()
        .envs(&spec.env)
        .env(ENV_HOME, HOME)
        .env(ENV_PATH, PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(output))
        .process_group(0)
        .fd_mappings(vec![
            FdMapping {
                parent_fd: commands_read.into(),
                child_fd: COMMANDS_FD,
            },
            FdMapping {
                parent_fd: messages_write.into(),
                child_fd: MESSAGES_FD,
            },
        ])?;
    if let Some(xvfb) = &xvfb {
        command
            .env(ENV_DISPLAY, xvfb.display())
            .env(ENV_XAUTHORITY, xvfb.auth_file());
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("cannot start the browser {program}"))?;
    // The command owns our copy of the browser's fd 4; while it lives the reader never sees EOF.
    drop(command);
    let group = child
        .id()
        .and_then(|pid| i32::try_from(pid).ok())
        .map(Pid::from_raw);

    commands_write.set_nonblocking(true)?;
    messages_read.set_nonblocking(true)?;
    let mut commands = UnixStream::from_std(commands_write)?;
    let mut messages = BufReader::with_capacity(BUFFER_BYTES, UnixStream::from_std(messages_read)?);
    let (mut from_brr, mut to_brr) = tokio::io::split(pipe);
    let flushed = link.clone();
    let mut outbound = tokio::spawn(async move {
        tokio::io::copy_buf(&mut messages, &mut to_brr).await?;
        to_brr.shutdown().await?;
        // The runner exits right after the browser; what it wrote must reach brr first.
        flushed.flush().await
    });

    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let ending = tokio::select! {
        result = tokio::io::copy(&mut from_brr, &mut commands) => {
            if let Err(error) = result {
                tracing::warn!(%error, "the command stream from brr failed");
            }
            Ending::Stop
        }
        _ = terminate.recv() => Ending::Stop,
        _ = interrupt.recv() => Ending::Stop,
        result = &mut outbound => match result {
            Ok(Ok(())) => Ending::Closed,
            Ok(Err(error)) => {
                tracing::warn!(%error, "cannot write browser messages to brr");
                Ending::Stop
            }
            Err(_) => Ending::Stop,
        },
        status = child.wait() => Ending::Exited(status),
    };
    let exited = matches!(ending, Ending::Exited(_));
    // A stop reaches the browser as EOF on its command pipe as well as by signal.
    drop(commands);

    let code = shut_down(&mut child, group, ending, STOP_GRACE).await;
    if exited && let Ok(Err(error)) = outbound.await {
        tracing::warn!(%error, "cannot write browser messages to brr");
    }
    if let Some(dir) = &spec.copy_back
        && let Err(error) = send_profile(&link, dir).await
    {
        tracing::warn!(%error, "the Profile was not sent back to brr");
    }

    if let Some(xvfb) = xvfb.as_mut() {
        xvfb.stop().await;
    }

    std::process::exit(code)
}

// SIGTERM lets the browser shut down cleanly where SIGKILL loses what it has not written yet.
async fn shut_down(child: &mut Child, group: Option<Pid>, ending: Ending, grace: Duration) -> i32 {
    let code = match ending {
        Ending::Exited(status) => exit_code(status),
        Ending::Closed => exit_code(wait_or_kill(child, group, grace).await),
        Ending::Stop => {
            if let Some(group) = group {
                let _ = killpg(group, Signal::SIGTERM);
            }
            let _ = wait_or_kill(child, group, grace).await;
            0
        }
    };
    // The browser's own children may outlive it in its group.
    if let Some(group) = group {
        let _ = killpg(group, Signal::SIGKILL);
    }

    code
}

async fn wait_or_kill(
    child: &mut Child,
    group: Option<Pid>,
    grace: Duration,
) -> io::Result<ExitStatus> {
    if let Ok(status) = tokio::time::timeout(grace, child.wait()).await {
        return status;
    }
    tracing::warn!(
        grace_s = grace.as_secs(),
        "the browser did not exit in time; killing it"
    );
    if let Some(group) = group {
        let _ = killpg(group, Signal::SIGKILL);
    }

    child.wait().await
}

fn exit_code(status: io::Result<ExitStatus>) -> i32 {
    status.map_or(FAILURE, |status| {
        status
            .code()
            .or_else(|| status.signal().map(|signal| SIGNAL_EXIT_BASE + signal))
            .unwrap_or(FAILURE)
    })
}

// brr's first stream on every run.
async fn receive_spec(accepted: &mut mpsc::Receiver<Stream>) -> anyhow::Result<RunnerSpec> {
    let stream = accepted.recv().await.with_context(|| {
        format!("brr sent no runner spec; `brr {NAME}` runs only inside a brr sandbox container")
    })?;
    if !matches!(serde_json::from_slice(stream.request()), Ok(Request::Spec)) {
        bail!("brr's first stream is not the runner spec; {IMAGE_HELP}");
    }

    let mut json = Vec::new();
    stream
        .take(MAX_SPEC_BYTES)
        .read_to_end(&mut json)
        .await
        .context("cannot read the runner spec from brr")?;

    parse(&json, VERSION)
}

// brr's streams after the spec, one at a time in the order brr opened them, so Start comes
// after every put is unpacked.
async fn serve_files(
    mut accepted: mpsc::Receiver<Stream>,
    writable: Vec<PathBuf>,
    started: oneshot::Sender<()>,
) {
    let mut started = Some(started);
    while let Some(stream) = accepted.recv().await {
        match serde_json::from_slice(stream.request()) {
            Ok(Request::Put(dir)) => put(stream, dir, &writable).await,
            Ok(Request::Get(path)) => get(stream, &path, &writable).await,
            Ok(Request::Start) => {
                if let Some(started) = started.take() {
                    let _ = started.send(());
                }
            }
            _ => tracing::warn!("brr opened a stream the runner does not know; refused"),
        }
    }
}

async fn put(stream: Stream, dir: PathBuf, writable: &[PathBuf]) {
    let (archive, mut answer) = tokio::io::split(stream);
    let unpacked = if files::inside(&dir, writable) {
        files::unpack(archive, dir)
            .await
            .map_err(|error| error.to_string())
    } else {
        Err(format!("{} is outside the run's folders", dir.display()))
    };

    let text = match unpacked {
        Ok(()) => PUT_DONE.to_owned(),
        Err(error) => error,
    };
    let _ = answer.write_all(text.as_bytes()).await;
    let _ = answer.shutdown().await;
}

// A failure closes the stream before the size, which brr reports as a failed fetch.
async fn get(mut stream: Stream, path: &Path, writable: &[PathBuf]) {
    if let Err(error) = send_file(&mut stream, path, writable).await {
        tracing::warn!(%error, path = %path.display(), "file not sent to brr");
    }
    let _ = stream.shutdown().await;
}

async fn send_file(stream: &mut Stream, path: &Path, writable: &[PathBuf]) -> io::Result<()> {
    if !files::inside(path, writable) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the path is outside the run's folders",
        ));
    }
    // The browser could have left a link to another file of the container.
    if !tokio::fs::symlink_metadata(path).await?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the path is not a regular file",
        ));
    }

    let file = tokio::fs::File::open(path).await?;
    let size = file.metadata().await?.len();
    stream.write_u64(size).await?;
    tokio::io::copy(&mut file.take(size), stream).await?;

    Ok(())
}

async fn send_profile(link: &Link, dir: &Path) -> anyhow::Result<()> {
    let stream = link.open(&serde_json::to_vec(&Request::Profile)?)?;
    let mut stream = files::pack(dir.to_owned(), stream).await?;
    stream.shutdown().await?;

    // The runner exits next; the archive must reach brr first.
    link.flush().await?;

    Ok(())
}
