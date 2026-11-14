use std::time::Duration;

use brr_core::Error;
use nix::sys::prctl::set_pdeathsig;
use nix::sys::signal::Signal;
use tokio::process::Command;

pub(crate) const EVENTS_QUEUE: usize = 1024;
pub(crate) const CONNECTION_CLOSED: &str = brr_cdp::CONNECTION_CLOSED;
pub(crate) const BLANK_PAGE: &str = "about:blank";
pub(crate) const ENV_INSTANCE_ID: &str = "BRR_INSTANCE_ID";
pub(crate) const ENV_DAEMON: &str = "BRR_DAEMON";
pub(crate) const ENV_TZ: &str = "TZ";
pub(crate) const ENV_DISPLAY: &str = "DISPLAY";
pub(crate) const ENV_WAYLAND_DISPLAY: &str = "WAYLAND_DISPLAY";
pub(crate) const ENV_XAUTHORITY: &str = "XAUTHORITY";

pub(crate) fn internal(message: impl ToString) -> Error {
    Error::Internal {
        message: message.to_string(),
    }
}

pub(crate) fn gone(what: &str) -> Error {
    Error::NotFound {
        what: what.to_owned(),
    }
}

pub(crate) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

// A browser must not outlive the daemon that supervises it, whatever way the daemon dies. The
// signal follows the spawning thread, which for tokio is a runtime worker that lives as long as
// the daemon.
pub(crate) fn die_with_parent(command: &mut Command) -> &mut Command {
    // SAFETY: the closure only makes the prctl syscall, which is async-signal-safe.
    unsafe {
        command.pre_exec(|| set_pdeathsig(Signal::SIGKILL).map_err(std::io::Error::from));
    }

    command
}
