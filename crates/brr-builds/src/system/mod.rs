use std::{
    ffi::OsStr,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tokio::{io::AsyncReadExt, process::Command, time::timeout};
use tracing::debug;

const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION_OUTPUT_CAP_BYTES: u64 = 4096;
const EXECUTABLE_BITS: u32 = 0o111;
const CHROME_NAME: &str = "Google Chrome";
const FIXED_CHROME_PATH: &str = "/opt/google/chrome/chrome";

const CANDIDATES: [(&str, &str); 9] = [
    ("google-chrome-stable", "Google Chrome"),
    ("google-chrome", "Google Chrome"),
    ("google-chrome-beta", "Google Chrome"),
    ("chromium", "Chromium"),
    ("chromium-browser", "Chromium"),
    ("brave-browser", "Brave"),
    ("brave", "Brave"),
    ("microsoft-edge-stable", "Microsoft Edge"),
    ("microsoft-edge", "Microsoft Edge"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct Detected {
    pub name: String,
    pub path: PathBuf,
    pub version: String,
}

pub async fn detect_system(path_env: &OsStr) -> Vec<Detected> {
    let mut found = Vec::<Detected>::new();

    for (file, name) in CANDIDATES {
        register(&mut found, find_in_path(file, path_env), name).await;
    }
    // The vendor install is a fallback; a PATH entry usually symlinks to it.
    if !found.iter().any(|detected| detected.name == CHROME_NAME) {
        register(
            &mut found,
            canonical_executable(Path::new(FIXED_CHROME_PATH)),
            CHROME_NAME,
        )
        .await;
    }

    found
}

async fn register(found: &mut Vec<Detected>, path: Option<PathBuf>, name: &str) {
    let Some(path) = path else { return };
    if found.iter().any(|detected| detected.path == path) {
        return;
    }

    let Some(version) = run_version(&path).await.as_deref().and_then(parse_version) else {
        debug!(path = %path.display(), "no parsable version, skipping browser candidate");
        return;
    };
    found.push(Detected {
        name: name.to_owned(),
        path,
        version,
    });
}

pub(crate) fn parse_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|token| token.contains('.') && token.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .map(str::to_owned)
}

fn find_in_path(file: &str, path_env: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_env).find_map(|dir| canonical_executable(&dir.join(file)))
}

fn canonical_executable(path: &Path) -> Option<PathBuf> {
    let canonical = path.canonicalize().ok()?;
    let metadata = canonical.metadata().ok()?;

    (metadata.is_file() && metadata.permissions().mode() & EXECUTABLE_BITS != 0)
        .then_some(canonical)
}

async fn run_version(path: &Path) -> Option<String> {
    let mut child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;

    let read = async {
        let mut raw = Vec::new();
        stdout
            .take(VERSION_OUTPUT_CAP_BYTES)
            .read_to_end(&mut raw)
            .await
            .ok()?;
        child.wait().await.ok()?;

        Some(raw)
    };
    let raw = timeout(VERSION_TIMEOUT, read).await.ok()??;

    Some(
        String::from_utf8_lossy(&raw)
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect(),
    )
}

#[cfg(test)]
mod tests;
