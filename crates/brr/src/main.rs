use std::{path::Path, process::ExitCode, time::Duration};

use brr_core::{DisplayEnv, EXIT_DAEMON_UNREACHABLE, Error, Paths, settings};
use brr_daemon::{Boot, remote::DAEMON_NAME};
use clap::ArgMatches;
use tracing::Level;

mod cli;
mod logging;

const ENV_PREFIX: &str = "BRR_";
const DEFAULT_ARGV0: &str = "brr";
const LEASE_HELD_HELP: &str = "pass --tab-id of a tab you hold, or use a read-only command";
const FAILURE: u8 = 1;

// Tokio's default pool grows to 512 threads under a request burst and keeps each one's stack and
// malloc arena for its 10 s keep-alive; blocking work here is short file and /proc reads.
const MAX_BLOCKING_THREADS: usize = 32;
const BLOCKING_KEEP_ALIVE: Duration = Duration::from_secs(2);

// glibc opens one malloc arena per busy thread (up to 8 per core); after a request burst the 24
// workers' arenas kept ~90 MB of freed memory resident. Four cost no measurable throughput. It
// must be set before the runtime spawns its threads.
#[cfg(target_env = "gnu")]
const ARENA_MAX: libc::c_int = 4;

fn main() -> ExitCode {
    let matches = cli::command().get_matches();
    #[cfg(target_env = "gnu")]
    if is_daemon_run(&matches) {
        // SAFETY: mallopt only sets an allocator tunable; no other thread exists yet.
        unsafe {
            libc::mallopt(libc::M_ARENA_MAX, ARENA_MAX);
        }
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .thread_keep_alive(BLOCKING_KEEP_ALIVE)
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return report(&anyhow::Error::from(error).context("cannot start the async runtime"));
        }
    };

    runtime.block_on(async {
        match start(&matches).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => report(&error),
        }
    })
}

fn is_daemon_run(matches: &ArgMatches) -> bool {
    matches
        .subcommand_matches(cli::DAEMON)
        .and_then(ArgMatches::subcommand_name)
        == Some("run")
}

async fn start(matches: &ArgMatches) -> anyhow::Result<()> {
    let boot = boot()?;

    let command = matches.subcommand_name();
    let foreground_daemon = is_daemon_run(matches);
    let base = if foreground_daemon || command == Some(cli::MCP) {
        Level::INFO
    } else {
        Level::WARN
    };
    let level = logging::level(
        base,
        matches.get_flag(cli::QUIET),
        matches.get_count(cli::VERBOSE),
    );
    // stdout carries the protocol for `mcp`, so its log goes to a file.
    let log_file = (command == Some(cli::MCP)).then(|| boot.paths.mcp_log());
    logging::init(
        level,
        std::env::var("RUST_LOG").ok().as_deref(),
        log_file.as_deref(),
    )?;

    let argv0 = std::env::args_os()
        .next()
        .as_deref()
        .and_then(|argv0| Path::new(argv0).file_name())
        .map_or_else(
            || DEFAULT_ARGV0.to_string(),
            |name| name.to_string_lossy().into_owned(),
        );

    cli::run(boot, matches, &argv0).await
}

// The only place that reads the environment; everything below gets Boot.
fn boot() -> Result<Boot, Error> {
    let paths = Paths::from_env(|key| std::env::var_os(key), nix::unistd::getuid().as_raw());
    let env: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(key, _)| key.starts_with(ENV_PREFIX))
        .collect();
    let (settings, sources) = settings::load(&paths.config_file(), &env)?;

    Ok(Boot {
        paths,
        settings,
        sources,
        env,
        // `DISPLAY= brr daemon run` sets an empty value; a browser cannot use it either.
        display_env: DisplayEnv {
            display: std::env::var_os("DISPLAY").filter(|value| !value.is_empty()),
            wayland_display: std::env::var_os("WAYLAND_DISPLAY").filter(|value| !value.is_empty()),
        },
        path_env: std::env::var_os("PATH").unwrap_or_default(),
        cft_base_url: brr_builds::CFT_BASE_URL.to_string(),
        version: env!("CARGO_PKG_VERSION"),
    })
}

fn report(error: &anyhow::Error) -> ExitCode {
    let typed = error.downcast_ref::<Error>();

    let (text, help, code) = match typed {
        Some(typed @ Error::DependencyMissing { name, help }) => {
            let code = if name == DAEMON_NAME {
                EXIT_DAEMON_UNREACHABLE
            } else {
                typed.exit_code()
            };

            (
                format!("{name} is not available"),
                Some(help.as_str()),
                code,
            )
        }
        Some(typed @ Error::LeaseHeld { .. }) => {
            (typed.to_string(), Some(LEASE_HELD_HELP), typed.exit_code())
        }
        Some(typed) => (typed.to_string(), None, typed.exit_code()),
        None => (format!("{error:#}"), None, FAILURE),
    };

    let help: Vec<&str> = help.into_iter().collect();
    let _ = cli::format::error(&text, &help);

    ExitCode::from(code)
}
