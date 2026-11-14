use std::{
    fs::{self, OpenOptions},
    io::{self, IsTerminal},
    path::Path,
    sync::Mutex,
};

use anyhow::Context;
use tracing::Level;
use tracing_subscriber::EnvFilter;

const LEVELS: [Level; 5] = [
    Level::ERROR,
    Level::WARN,
    Level::INFO,
    Level::DEBUG,
    Level::TRACE,
];

pub fn level(base: Level, quiet: bool, verbose: u8) -> Level {
    let base = LEVELS.iter().position(|level| *level == base).unwrap_or(0);
    let index = (base + usize::from(verbose))
        .saturating_sub(usize::from(quiet))
        .min(LEVELS.len() - 1);

    LEVELS[index]
}

// RUST_LOG replaces the level when it parses. `log_file` is for stdio commands, where stderr is
// not free either; everything else logs to stderr.
pub fn init(level: Level, rust_log: Option<&str>, log_file: Option<&Path>) -> anyhow::Result<()> {
    let filter = rust_log
        .and_then(|directives| EnvFilter::try_new(directives).ok())
        .unwrap_or_else(|| EnvFilter::new(level.as_str()));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);

    match log_file {
        Some(path) => {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .with_context(|| format!("cannot open {}", path.display()))?;

            builder
                .with_writer(Mutex::new(file))
                .with_ansi(false)
                .init();
        }
        None => builder
            .with_writer(io::stderr)
            .with_ansi(io::stderr().is_terminal())
            .init(),
    }

    Ok(())
}
