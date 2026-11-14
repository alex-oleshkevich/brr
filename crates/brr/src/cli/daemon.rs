use std::{
    fs::{self, OpenOptions},
    io,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command as Process, Stdio},
    time::Duration,
};

use anyhow::{Context, bail};
use brr_core::{Paths, Settings};
use brr_daemon::{
    App, Boot,
    remote::{Remote, daemon_missing},
};
use clap::{Arg, ArgMatches, Command, value_parser};
use nix::{
    sys::signal::{self, Signal},
    unistd::{Pid, setsid},
};
use serde_json::json;
use tokio::time;

use super::{DAEMON, files, format, render, systemd};

const LINES: &str = "lines";
const DEFAULT_LINES: &str = "100";
const MAX_STDERR_CHARS: usize = 300;
const SYSTEMD_NAME: &str = "brr";
const POLL: Duration = Duration::from_millis(100);
const AUTOSTART_TIMEOUT: Duration = Duration::from_secs(10);
const SYSTEM_INFO: &str = "system.info";

pub fn command() -> Command {
    Command::new(DAEMON)
        .about("Run and manage the daemon")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(Command::new("run").about("Run the daemon in the foreground"))
        .subcommand(Command::new("start").about("Start the daemon in the background"))
        .subcommand(Command::new("stop").about("Stop the running daemon"))
        .subcommand(Command::new("status").about("Show the daemon's system info"))
        .subcommand(Command::new("install").about("Install and start the systemd user service"))
        .subcommand(Command::new("uninstall").about("Stop and remove the systemd user service"))
        .subcommand(
            Command::new("logs")
                .about("Print the end of the daemon log")
                .arg(
                    Arg::new(LINES)
                        .short('n')
                        .long(LINES)
                        .value_parser(value_parser!(usize))
                        .default_value(DEFAULT_LINES)
                        .help("How many lines to print"),
                ),
        )
}

pub async fn run(boot: Boot, matches: &ArgMatches, json: bool) -> anyhow::Result<()> {
    match matches.subcommand() {
        Some(("run", _)) => {
            let app = App::start(boot).await?;

            Ok(brr_daemon::run(app).await?)
        }
        Some(("start", _)) => start(&boot.paths, &boot.settings).await,
        Some(("stop", _)) => stop(&boot.paths, &boot.settings).await,
        Some(("status", _)) => status(&boot.paths, &boot.settings, json).await,
        Some(("install", _)) => install(&boot).await,
        Some(("uninstall", _)) => uninstall(&boot.paths).await,
        Some(("logs", logs)) => {
            let lines = logs.get_one::<usize>(LINES).copied().unwrap_or_default();

            print_logs(&boot.paths, lines)
        }
        _ => bail!("unknown daemon command"),
    }
}

// Every CLI command except `daemon *` starts the daemon when its socket is absent.
pub async fn ensure_daemon(paths: &Paths, settings: &Settings) -> anyhow::Result<Remote> {
    let remote = Remote::new(&paths.socket(settings))?;
    if !remote.is_up().await {
        launch(&remote, paths).await?;
    }

    Ok(remote)
}

async fn launch(remote: &Remote, paths: &Paths) -> anyhow::Result<()> {
    let unreachable =
        |cause: String| daemon_missing(format!("{cause}; see {}", paths.daemon_log().display()));

    let mut child = spawn(paths).map_err(|cause| unreachable(format!("{cause:#}")))?;

    let up = async {
        loop {
            if remote.is_up().await {
                return Ok(());
            }
            if let Some(status) = child
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten())
            {
                return Err(unreachable(format!("the daemon exited with {status}")));
            }
            time::sleep(POLL).await;
        }
    };

    match time::timeout(AUTOSTART_TIMEOUT, up).await {
        Ok(started) => Ok(started?),
        Err(_) => Err(unreachable("the daemon did not answer in time".to_string()).into()),
    }
}

// None under systemd: the unit owns the process.
fn spawn(paths: &Paths) -> anyhow::Result<Option<Child>> {
    if unit_file(paths).exists() {
        systemctl(&["start", SYSTEMD_NAME])?;

        return Ok(None);
    }

    fs::create_dir_all(&paths.state_dir)?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.daemon_log())
        .context("cannot open the daemon log")?;

    let mut daemon = Process::new(std::env::current_exe()?);
    daemon
        .args([DAEMON, "run"])
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // SAFETY: setsid is async-signal-safe and the closure touches no shared state; the daemon must
    // outlive the caller's session and process group, so a terminal hangup never reaches it.
    unsafe {
        daemon.pre_exec(|| setsid().map(drop).map_err(io::Error::from));
    }

    Ok(Some(daemon.spawn().context("cannot start the daemon")?))
}

async fn start(paths: &Paths, settings: &Settings) -> anyhow::Result<()> {
    let remote = Remote::new(&paths.socket(settings))?;
    if remote.is_up().await {
        return Ok(format::status("Running", "daemon is already running")?);
    }

    launch(&remote, paths).await?;

    Ok(format::status("Started", "daemon")?)
}

async fn stop(paths: &Paths, settings: &Settings) -> anyhow::Result<()> {
    let remote = Remote::new(&paths.socket(settings))?;
    if !remote.is_up().await {
        return Ok(format::status("Skipped", "daemon is not running")?);
    }

    let lock_file = paths.lock_file();
    let pid = fs::read_to_string(&lock_file)
        .with_context(|| format!("cannot read {}", lock_file.display()))?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|pid| *pid != 0)
        .and_then(|pid| i32::try_from(pid).ok())
        .with_context(|| format!("{} does not hold a pid", lock_file.display()))?;
    // Pid 0 would signal the caller's whole process group.
    signal::kill(Pid::from_raw(pid), Signal::SIGTERM).context("cannot signal the daemon")?;

    let wait = Duration::from_secs(settings.instance.stop_timeout_s + systemd::STOP_GRACE_S);
    let down = async {
        while remote.is_up().await {
            time::sleep(POLL).await;
        }
    };
    if time::timeout(wait, down).await.is_err() {
        bail!("the daemon did not stop within {} s", wait.as_secs());
    }

    Ok(format::status("Stopped", "daemon")?)
}

async fn status(paths: &Paths, settings: &Settings, json: bool) -> anyhow::Result<()> {
    let remote = Remote::new(&paths.socket(settings))?;
    if !remote.is_up().await {
        return Err(daemon_missing("it is not running; start it with `brr daemon start`").into());
    }

    let info = remote.call(SYSTEM_INFO, &json!({}), None).await?;

    render::output(&info, json, None, None)
}

fn print_logs(paths: &Paths, lines: usize) -> anyhow::Result<()> {
    let log: &Path = &paths.daemon_log();
    let tail =
        files::tail_lines(log, lines).with_context(|| format!("cannot read {}", log.display()))?;

    for line in &tail {
        format::data(line)?;
    }

    Ok(())
}

fn unit_file(paths: &Paths) -> PathBuf {
    systemd::unit_path(paths.config_dir.parent().unwrap_or(Path::new("")))
}

// Argv only, no shell.
fn systemctl(args: &[&str]) -> anyhow::Result<()> {
    let output = Process::new("systemctl")
        .arg("--user")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("cannot run systemctl")?;
    if output.status.success() {
        return Ok(());
    }

    let stderr: String = String::from_utf8_lossy(&output.stderr)
        .trim()
        .chars()
        .take(MAX_STDERR_CHARS)
        .collect();

    bail!("`systemctl --user {}` failed: {stderr}", args.join(" "))
}

async fn install(boot: &Boot) -> anyhow::Result<()> {
    let unit = unit_file(&boot.paths);
    let paths = &boot.paths;
    let xdg = [
        ("XDG_CONFIG_HOME", base_dir(&paths.config_dir)),
        ("XDG_DATA_HOME", base_dir(&paths.data_dir)),
        ("XDG_STATE_HOME", base_dir(&paths.state_dir)),
    ];
    let text = systemd::render_unit(&systemd::UnitInput {
        exe: &std::env::current_exe()?,
        path_env: &boot.path_env,
        daemon_log: &paths.daemon_log(),
        stop_timeout_s: boot.settings.instance.stop_timeout_s,
        xdg: &xdg,
    });

    if let Some(dir) = unit.parent() {
        fs::create_dir_all(dir)?;
    }
    let temp = unit.with_extension("service.tmp");
    fs::write(&temp, text).and_then(|()| fs::rename(&temp, &unit))?;

    // The file alone makes every autostart go through systemctl, so a unit that did not register
    // must not stay behind.
    if let Err(error) =
        systemctl(&["daemon-reload"]).and_then(|()| systemctl(&["enable", SYSTEMD_NAME]))
    {
        let _ = fs::remove_file(&unit);

        return Err(error);
    }

    // A daemon started by hand holds the socket and the lock; systemd must own the next one.
    let remote = Remote::new(&paths.socket(&boot.settings))?;
    if remote.is_up().await {
        stop(paths, &boot.settings).await?;
    }
    launch(&remote, paths).await?;

    Ok(format::status("Installed", &unit.display().to_string())?)
}

async fn uninstall(paths: &Paths) -> anyhow::Result<()> {
    let unit = unit_file(paths);
    if !unit.exists() {
        return Ok(format::status("Skipped", "the service is not installed")?);
    }

    // The file goes even when systemd could not disable the unit, so autostart works again.
    let disabled = systemctl(&["disable", "--now", SYSTEMD_NAME]);
    fs::remove_file(&unit)?;
    let reloaded = systemctl(&["daemon-reload"]);
    disabled.and(reloaded)?;

    Ok(format::status("Uninstalled", &unit.display().to_string())?)
}

fn base_dir(app_dir: &Path) -> &Path {
    app_dir.parent().unwrap_or(app_dir)
}
