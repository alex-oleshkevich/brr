use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use brr_core::{Error, InstanceId, Size};
use command_fds::{CommandFdExt, FdMapping};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use tokio::io::AsyncReadExt;
use tokio::net::unix::pipe::Receiver;
use tokio::process::{Child, Command};

use crate::common::{
    ENV_DAEMON, ENV_DISPLAY, ENV_INSTANCE_ID, ENV_WAYLAND_DISPLAY, ENV_XAUTHORITY, die_with_parent,
    internal, millis,
};

#[cfg(test)]
mod tests;

pub const DEFAULT_VIRTUAL_SCREEN: Size = Size {
    width: 1920,
    height: 1080,
};

const XVFB: &str = "Xvfb";
const SCREEN_FIELD: &str = "screen";
const INSTALL_HELP: &str =
    "install the Xvfb package (xorg-server-xvfb, or xvfb on Debian) and retry";
// A real display is never smaller; 1x1 is what a forgotten size looks like.
const MIN_SCREEN_SIDE: u32 = 100;
const COLOR_DEPTH: u32 = 24;
const DISPLAY_FD: i32 = 3;
const START_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const AUTH_FILE_MODE: u32 = 0o600;
// The Xauthority entry format: a 16-bit family, then four length-prefixed strings.
const FAMILY_WILD: u16 = 0xFFFF;
const AUTH_NAME: &[u8] = b"MIT-MAGIC-COOKIE-1";
const COOKIE_BYTES: usize = 16;

// One Xvfb per Instance; the driver that started it stops it when the browser is gone.
pub struct Xvfb {
    child: Child,
    display: String,
    auth_file: PathBuf,
}

// A server nobody authenticates to is open to every local user, who could read the screen and
// the keystrokes of an agent session. The server loads only the name and data of an entry, and a
// wildcard family and number make the same entry match any display for the clients, so it can
// be written before the display number is known.
fn write_auth_file(path: &Path) -> Result<(), Error> {
    let mut entry = Vec::new();
    entry.extend(FAMILY_WILD.to_be_bytes());
    entry.extend(0_u16.to_be_bytes());
    entry.extend(0_u16.to_be_bytes());
    entry.extend((AUTH_NAME.len() as u16).to_be_bytes());
    entry.extend(AUTH_NAME);
    entry.extend((COOKIE_BYTES as u16).to_be_bytes());
    entry.extend(rand::random::<[u8; COOKIE_BYTES]>());

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(AUTH_FILE_MODE)
        .open(path)
        .map_err(internal)?;

    file.write_all(&entry).map_err(internal)
}

pub(crate) fn args(screen: &Size, auth_file: &Path) -> Result<Vec<OsString>, Error> {
    if screen.width < MIN_SCREEN_SIDE || screen.height < MIN_SCREEN_SIDE {
        return Err(Error::InvalidArgument {
            field: SCREEN_FIELD.to_owned(),
            message: format!(
                "a virtual screen is at least {MIN_SCREEN_SIDE}x{MIN_SCREEN_SIDE}; got {}x{}",
                screen.width, screen.height
            ),
        });
    }

    let mut args: Vec<OsString> = [
        "-displayfd",
        &DISPLAY_FD.to_string(),
        "-screen",
        "0",
        &format!("{}x{}x{COLOR_DEPTH}", screen.width, screen.height),
        "-nolisten",
        "tcp",
        "-noreset",
        "-auth",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(auth_file.into());

    Ok(args)
}

impl Xvfb {
    pub async fn start(
        screen: Size,
        instance_id: &InstanceId,
        log_file: &Path,
    ) -> Result<Xvfb, Error> {
        let auth_file = log_file.with_file_name(format!("{instance_id}.xauth"));
        let args = args(&screen, &auth_file)?;
        if let Some(dir) = log_file.parent() {
            std::fs::create_dir_all(dir).map_err(internal)?;
        }
        write_auth_file(&auth_file)?;
        // A file, never a pipe: an unread full pipe blocks the server.
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file)
            .map_err(internal)?;
        let log_err = log.try_clone().map_err(internal)?;
        let (reader, writer) = std::io::pipe().map_err(internal)?;

        let mut command = Command::new(XVFB);
        command
            .args(args)
            .env(ENV_INSTANCE_ID, instance_id.to_string())
            .env(ENV_DAEMON, "1")
            // The server must not inherit the host's displays.
            .env_remove(ENV_WAYLAND_DISPLAY)
            .env_remove(ENV_DISPLAY)
            .env_remove(ENV_XAUTHORITY)
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(log_err)
            .process_group(0)
            .kill_on_drop(true)
            .fd_mappings(vec![FdMapping {
                parent_fd: writer.into(),
                child_fd: DISPLAY_FD,
            }])
            .map_err(internal)?;
        die_with_parent(&mut command);
        let spawned = command.spawn().map_err(|error| match error.kind() {
            ErrorKind::NotFound => Error::DependencyMissing {
                name: XVFB.to_owned(),
                help: INSTALL_HELP.to_owned(),
            },
            _ => internal(error),
        });
        let child = match spawned {
            Ok(child) => child,
            Err(error) => {
                let _ = std::fs::remove_file(&auth_file);
                return Err(error);
            }
        };
        // The command owns the parent's copy of the write end; while it lives the read below
        // never sees EOF if Xvfb dies.
        drop(command);

        let mut xvfb = Xvfb {
            child,
            display: String::new(),
            auth_file,
        };
        match tokio::time::timeout(START_TIMEOUT, read_display(reader.into())).await {
            Ok(Ok(number)) => {
                xvfb.display = format!(":{number}");
                Ok(xvfb)
            }
            Ok(Err(error)) => {
                xvfb.stop().await;
                Err(error)
            }
            Err(_) => {
                xvfb.stop().await;
                Err(Error::Timeout {
                    after_ms: millis(START_TIMEOUT),
                })
            }
        }
    }

    pub fn display(&self) -> &str {
        &self.display
    }

    pub fn auth_file(&self) -> &Path {
        &self.auth_file
    }

    pub fn pid(&self) -> u32 {
        self.child.id().unwrap_or_default()
    }

    // SIGTERM to the group, SIGKILL if it lingers, and the child is reaped either way.
    pub async fn stop(&mut self) {
        if let Some(pid) = self.child.id().and_then(|pid| i32::try_from(pid).ok()) {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGTERM);
        }
        if tokio::time::timeout(STOP_TIMEOUT, self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
        let _ = std::fs::remove_file(&self.auth_file);
    }
}

// -displayfd writes the display number and a newline once the server accepts connections.
async fn read_display(fd: OwnedFd) -> Result<u32, Error> {
    let mut receiver = Receiver::from_owned_fd(fd).map_err(internal)?;
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];

    loop {
        if receiver.read(&mut byte).await.map_err(internal)? == 0 {
            return Err(internal("Xvfb exited before it reported its display"));
        }
        if byte[0] == b'\n' {
            break;
        }
        line.push(byte[0]);
    }

    String::from_utf8_lossy(&line)
        .trim()
        .parse()
        .map_err(|_| internal("Xvfb reported an unreadable display number"))
}
