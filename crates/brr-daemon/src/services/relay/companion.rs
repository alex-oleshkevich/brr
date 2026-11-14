use std::{
    fs,
    io::{Cursor, Write},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use brr_core::{Capability, Error, ProfileSettings, sha256_hex};
use brr_store::Connection;
use rust_embed::Embed;
use serde_json::json;
use zip::{ZipWriter, write::SimpleFileOptions};

use crate::{app::App, internal, services::profiles::resolve_build};

const ZIP_NAME: &str = "brr-companion.zip";
const CONFIG_FILE: &str = "brr.json";
const RELAY_PATH: &str = "/v1/extension-relay";
const HASH_CHARS: usize = 16;

static PARTIALS: AtomicU64 = AtomicU64::new(0);

// Present only when the binary was built after `just ext-build`.
#[derive(Embed)]
#[folder = "../../extension/dist"]
#[allow_missing = true]
struct Dist;

pub const FILENAME: &str = ZIP_NAME;

fn relay_url(bind: SocketAddr) -> String {
    let host = if bind.ip().is_unspecified() {
        IpAddr::from([127, 0, 0, 1])
    } else {
        bind.ip()
    };

    format!("ws://{}{RELAY_PATH}", SocketAddr::new(host, bind.port()))
}

// The embedded files plus the brr.json that points the extension at this daemon, sorted by name.
fn files(app: &App) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let mut files: Vec<(String, Vec<u8>)> = Dist::iter()
        .filter_map(|name| Some((name.to_string(), Dist::get(&name)?.data.into_owned())))
        .collect();
    if files.is_empty() {
        return Err(Error::DependencyMissing {
            name: "companion extension".to_owned(),
            help:
                "this brr was built without the companion extension; run just ext-build and rebuild"
                    .to_owned(),
        });
    }

    let config = json!({ "relay_url": relay_url(app.settings().server.bind) });
    files.push((CONFIG_FILE.to_owned(), config.to_string().into_bytes()));
    files.sort();

    Ok(files)
}

pub fn zip(app: &App) -> Result<Vec<u8>, Error> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in files(app)? {
        writer
            .start_file(name, SimpleFileOptions::default())
            .map_err(internal)?;
        writer.write_all(&data).map_err(internal)?;
    }

    Ok(writer.finish().map_err(internal)?.into_inner())
}

// Content-addressed, so a new brr (or a new bind address) gets a fresh directory and the
// browser's loadUnpacked never sees a half-written one.
pub fn companion_dir(app: &App) -> Result<PathBuf, Error> {
    let files = files(app)?;
    let mut content = Vec::new();
    for (name, data) in &files {
        content.extend_from_slice(name.as_bytes());
        content.extend_from_slice(data);
    }
    let hash = sha256_hex(&content);

    let root = app.paths.data_dir.join("companion");
    let dir = root.join(&hash[..HASH_CHARS]);
    if dir.is_dir() {
        return Ok(dir);
    }

    // Unique per call: two launches of the first companion Profile must not share a partial copy.
    let partial = root.join(format!(
        ".{}-{}-{}",
        &hash[..HASH_CHARS],
        std::process::id(),
        PARTIALS.fetch_add(1, Ordering::Relaxed)
    ));
    write_files(&partial, &files)?;
    if let Err(error) = fs::rename(&partial, &dir) {
        let _ = fs::remove_dir_all(&partial);
        // A concurrent launch won the rename; its copy is identical.
        if !dir.is_dir() {
            return Err(internal(error));
        }
    }

    Ok(dir)
}

fn write_files(dir: &Path, files: &[(String, Vec<u8>)]) -> Result<(), Error> {
    let _ = fs::remove_dir_all(dir);
    for (name, data) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(internal)?;
        }
        fs::write(path, data).map_err(internal)?;
    }

    Ok(())
}

// E2a for the companion: saved on a Profile whose Build cannot load unpacked extensions it
// would only fail at launch. A channel without an installed Build is re-checked at launch.
pub fn check_settings(c: &Connection, settings: &ProfileSettings) -> Result<(), Error> {
    if !settings.companion {
        return Ok(());
    }

    match resolve_build(c, &settings.build) {
        Ok(found) => require_unpacked(&found),
        Err(Error::BuildMissing { .. }) => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn require_unpacked(found: &brr_core::Build) -> Result<(), Error> {
    if found.capabilities.contains(&Capability::UnpackedExtensions) {
        return Ok(());
    }

    Err(Error::EngineUnsupported {
        capability: Capability::UnpackedExtensions,
    })
}
