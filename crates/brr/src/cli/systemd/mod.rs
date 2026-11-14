use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

#[cfg(test)]
mod tests;

const UNIT_FILE: &str = "systemd/user/brr.service";
// systemd SIGKILLs what is left this long after the daemon's own shutdown budget; `daemon stop`
// waits the same grace.
pub const STOP_GRACE_S: u64 = 10;

pub struct UnitInput<'a> {
    pub exe: &'a Path,
    pub path_env: &'a OsStr,
    pub daemon_log: &'a Path,
    pub stop_timeout_s: u64,
    // The CLI's XDG base dirs, so the daemon uses the same config, data and state as the CLI.
    pub xdg: &'a [(&'a str, &'a Path)],
}

pub fn unit_path(config_home: &Path) -> PathBuf {
    config_home.join(UNIT_FILE)
}

pub fn render_unit(unit: &UnitInput<'_>) -> String {
    // ExecStart expands $VAR even in quotes; Environment does not.
    let exe = quoted(&unit.exe.to_string_lossy().replace('$', "$$"));
    let log = specifiers(&unit.daemon_log.to_string_lossy());
    let stop_timeout_s = unit.stop_timeout_s + STOP_GRACE_S;

    let environment: String = [("PATH", unit.path_env)]
        .into_iter()
        .chain(unit.xdg.iter().map(|(name, dir)| (*name, dir.as_os_str())))
        .map(|(name, value)| {
            format!(
                "Environment={}\n",
                quoted(&format!("{name}={}", value.to_string_lossy()))
            )
        })
        .collect();

    format!(
        "[Unit]\n\
         Description=brr browser manager\n\
         \n\
         [Service]\n\
         ExecStart={exe} daemon run\n\
         Restart=on-failure\n\
         PassEnvironment=DISPLAY WAYLAND_DISPLAY XDG_RUNTIME_DIR\n\
         {environment}\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         KillMode=mixed\n\
         TimeoutStopSec={stop_timeout_s}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

// A literal % would otherwise expand as a unit specifier.
fn specifiers(value: &str) -> String {
    value.replace('%', "%%")
}

// One quoted word; systemd decodes backslashes inside the quotes.
fn quoted(value: &str) -> String {
    let escaped = specifiers(value).replace('\\', "\\\\").replace('"', "\\\"");

    format!("\"{escaped}\"")
}
