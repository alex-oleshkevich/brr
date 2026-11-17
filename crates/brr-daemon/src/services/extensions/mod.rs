mod crx;
mod unpack;

#[cfg(test)]
mod tests;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use brr_core::{
    Build, BuildChoice, Capability, ConflictReason, Engine, Error, EventData, Extension,
    ExtensionId, ExtensionSource, ExtensionVersion, InstanceExtension, Page, Profile,
    ProfileExtension, ProfileSettings, compare_versions, now_ms, sha256_hex,
};
use brr_store::{Connection, build, extension, profile_extension};
use futures::future::join_all;
use reqwest::StatusCode;
use tokio::{task, time};
use tracing::warn;

use self::unpack::{Archive, is_gecko, unpack};
use crate::{
    app::App,
    internal,
    ops::invalid,
    services::{builds::engine_of, profiles::resolve_build},
};

pub const UPDATE_BUDGET: Duration = Duration::from_secs(5);
pub const MAX_EXTENSION_BYTES: u64 = 64 * 1024 * 1024;

const EXTENSIONS_DIR: &str = "extensions";
const DEFAULT_PRODVERSION: &str = "150.0.0.0";
const UPDATECHECK_MAX_BYTES: u64 = 1 << 20;
const AMO_META_MAX_BYTES: u64 = 1 << 20;
const CWS_ID_CHARS: usize = 32;
const SHA256_PREFIX: &str = "sha256:";
const TAR_MAGIC: &[u8] = b"ustar";
const TAR_MAGIC_OFFSET: usize = 257;
const ZIP_MAGIC: &[u8] = b"PK";
const CRX_MAGIC: &[u8] = b"Cr24";
const PAGE_LIMIT: u32 = 100;
const CWS_HOSTS: [&str; 2] = ["chromewebstore.google.com", "chrome.google.com"];
const AMO_HOST: &str = "addons.mozilla.org";
const STAGING_SUFFIX: &str = ".tmp";
const FETCH_STALL_TIMEOUT: Duration = Duration::from_secs(30);
const LOADED_BY_INSTANCE: &str = "a running instance has it loaded";
const REFERENCE_SEPARATOR: &str = ", ";
const CWS_SITE: &str = "the Chrome Web Store";
const AMO_SITE: &str = "Firefox Add-ons";
const UNREACHABLE_HELP: &str = "check the network connection, then retry";
// Present in every add-on addons.mozilla.org signed.
const MOZILLA_SIGNATURE: &str = "META-INF/mozilla.rsa";

pub enum AddSource {
    Cws {
        reference: String,
    },
    Amo {
        reference: String,
    },
    File {
        bytes: Vec<u8>,
        engine: Option<Engine>,
    },
}

#[derive(Clone, Copy)]
enum Kind {
    Crx,
    Zip,
    Tar,
}

struct AmoMeta {
    version: String,
    url: String,
    sha256: String,
}

pub fn root(app: &App) -> PathBuf {
    app.paths.data_dir.join(EXTENSIONS_DIR)
}

fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Chromium => "Chromium",
        Engine::Camoufox => "Camoufox",
    }
}

pub fn parse_cws_ref(reference: &str) -> Option<String> {
    let is_id =
        |id: &str| id.len() == CWS_ID_CHARS && id.bytes().all(|b| (b'a'..=b'p').contains(&b));
    if is_id(reference) {
        return Some(reference.to_string());
    }

    let rest = reference
        .strip_prefix("https://")
        .or_else(|| reference.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/')?;
    if !CWS_HOSTS.contains(&host) || !path.contains("detail/") {
        return None;
    }
    let last = path.split(['?', '#']).next()?.trim_end_matches('/');

    last.rsplit('/')
        .next()
        .filter(|id| is_id(id))
        .map(str::to_string)
}

pub fn parse_amo_ref(reference: &str, url_only: bool) -> Option<String> {
    let slug_ok = |slug: &str| {
        !slug.is_empty()
            && slug
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    };
    if let Some(rest) = reference
        .strip_prefix("https://")
        .or_else(|| reference.strip_prefix("http://"))
    {
        let (host, path) = rest.split_once('/')?;
        if host != AMO_HOST {
            return None;
        }
        let path = path.split(['?', '#']).next()?;
        let mut parts = path.split('/');
        let slug = parts.by_ref().skip_while(|part| *part != "addon").nth(1)?;

        return slug_ok(slug).then(|| slug.to_string());
    }

    (!url_only && slug_ok(reference)).then(|| reference.to_string())
}

// Upstream answers are mapped to what the caller can act on; the raw URL stays in the log.
async fn fetch(app: &App, url: &str, cap: u64, site: &str, item: &str) -> Result<Vec<u8>, Error> {
    let unreachable = |detail: String| {
        warn!(url, site, %detail, "extension source unreachable");

        Error::DependencyMissing {
            name: site.to_string(),
            help: UNREACHABLE_HELP.to_string(),
        }
    };
    let not_found = || Error::NotFound {
        what: format!("{item} on {site}"),
    };

    let mut response = time::timeout(FETCH_STALL_TIMEOUT, app.http.get(url).send())
        .await
        .map_err(|_| unreachable("no answer".to_string()))?
        .map_err(|e| unreachable(e.without_url().to_string()))?;
    let status = response.status();
    if matches!(status, StatusCode::NOT_FOUND | StatusCode::GONE) {
        return Err(not_found());
    }
    if !status.is_success() {
        return Err(unreachable(format!("HTTP {status}")));
    }
    let too_big = || internal(format!("{site} sent more than {cap} bytes for {item}"));
    if response.content_length().is_some_and(|len| len > cap) {
        return Err(too_big());
    }

    let mut body = Vec::new();
    loop {
        let chunk = time::timeout(FETCH_STALL_TIMEOUT, response.chunk())
            .await
            .map_err(|_| unreachable("stopped sending data".to_string()))?
            .map_err(|e| unreachable(e.without_url().to_string()))?;
        let Some(chunk) = chunk else {
            break;
        };
        if body.len() as u64 + chunk.len() as u64 > cap {
            return Err(too_big());
        }
        body.extend_from_slice(&chunk);
    }
    // The Web Store answers an unknown id with an empty body.
    if body.is_empty() {
        return Err(not_found());
    }

    Ok(body)
}

async fn prodversion(app: &Arc<App>) -> Result<String, Error> {
    let builds = app.store.run(|c| build::all(c)).await?;

    Ok(builds
        .into_iter()
        .filter(|b| b.engine == Engine::Chromium)
        .map(|b| b.version)
        .max_by(|a, b| compare_versions(a, b))
        .unwrap_or_else(|| DEFAULT_PRODVERSION.to_string()))
}

fn cws_url(app: &App, query: &str) -> String {
    format!("{}?{query}", app.settings().extensions.cws_update_url)
}

async fn amo_meta(app: &App, slug: &str) -> Result<AmoMeta, Error> {
    let url = format!(
        "{}/addons/addon/{slug}/",
        app.settings().extensions.amo_api.trim_end_matches('/')
    );
    let item = format!("add-on {slug}");
    let body = fetch(app, &url, AMO_META_MAX_BYTES, AMO_SITE, &item).await?;
    let meta: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| internal(format!("{AMO_SITE} answered with bad JSON: {e}")))?;
    let current = &meta["current_version"];
    let text = |value: &serde_json::Value| value.as_str().map(str::to_string);
    let missing = || {
        invalid(
            "ref",
            &format!("{AMO_SITE} has no downloadable version of {slug}"),
        )
    };

    Ok(AmoMeta {
        version: text(&current["version"]).ok_or_else(missing)?,
        url: text(&current["file"]["url"]).ok_or_else(missing)?,
        // Verification must not be skippable: a missing or other-algorithm hash is an error.
        sha256: text(&current["file"]["hash"])
            .and_then(|hash| hash.strip_prefix(SHA256_PREFIX).map(str::to_string))
            .ok_or_else(|| internal(format!("{AMO_SITE} gave no sha256 for {slug}")))?,
    })
}

async fn fetch_xpi(app: &App, meta: &AmoMeta, slug: &str) -> Result<Vec<u8>, Error> {
    let item = format!("add-on {slug}");
    let bytes = fetch(app, &meta.url, MAX_EXTENSION_BYTES, AMO_SITE, &item).await?;
    if !meta.sha256.eq_ignore_ascii_case(&sha256_hex(&bytes)) {
        return Err(internal("checksum mismatch for the downloaded add-on"));
    }

    Ok(bytes)
}

pub async fn add(app: &Arc<App>, source: AddSource) -> Result<Extension, Error> {
    match source {
        AddSource::Cws { reference } => {
            let id = parse_cws_ref(&reference).ok_or_else(|| {
                invalid(
                    "ref",
                    "use a 32-letter Web Store id or a chromewebstore.google.com/detail URL",
                )
            })?;
            let prodversion = prodversion(app).await?;
            let url = cws_url(
                app,
                &format!(
                    "response=redirect&prodversion={prodversion}&acceptformat=crx2,crx3&x=id%3D{id}%26uc"
                ),
            );
            let item = format!("extension {id}");
            let bytes = fetch(app, &url, MAX_EXTENSION_BYTES, CWS_SITE, &item).await?;

            install(app, ExtensionSource::Cws, id, bytes, None).await
        }
        AddSource::Amo { reference } => {
            let slug = parse_amo_ref(&reference, false).ok_or_else(|| {
                invalid(
                    "ref",
                    "use an add-on slug or an addons.mozilla.org add-on URL",
                )
            })?;
            let meta = amo_meta(app, &slug).await?;
            let bytes = fetch_xpi(app, &meta, &slug).await?;

            install(app, ExtensionSource::Amo, slug, bytes, None).await
        }
        AddSource::File { bytes, engine } => {
            let digest = sha256_hex(&bytes);
            let known = digest.clone();
            if let Some(existing) = app
                .store
                .run(move |c| extension::find_by_source(c, ExtensionSource::Local, &known))
                .await?
            {
                return Ok(existing);
            }

            install(app, ExtensionSource::Local, digest, bytes, engine).await
        }
    }
}

fn detect(bytes: &[u8]) -> Result<Kind, Error> {
    if bytes.starts_with(CRX_MAGIC) {
        Ok(Kind::Crx)
    } else if bytes.starts_with(ZIP_MAGIC) {
        Ok(Kind::Zip)
    } else if bytes.get(TAR_MAGIC_OFFSET..TAR_MAGIC_OFFSET + TAR_MAGIC.len()) == Some(TAR_MAGIC) {
        Ok(Kind::Tar)
    } else {
        Err(invalid(
            "file",
            "expected a .crx, .xpi or .zip file, or an uploaded folder",
        ))
    }
}

async fn install(
    app: &Arc<App>,
    source: ExtensionSource,
    source_ref: String,
    bytes: Vec<u8>,
    engine: Option<Engine>,
) -> Result<Extension, Error> {
    if bytes.len() as u64 > MAX_EXTENSION_BYTES {
        return Err(invalid(
            "file",
            &format!("larger than {MAX_EXTENSION_BYTES} bytes"),
        ));
    }
    let kind = detect(&bytes)?;
    let lookup = source_ref.clone();
    let existing = app
        .store
        .run(move |c| extension::find_by_source(c, source, &lookup))
        .await?;
    let extension_id = existing.as_ref().map_or_else(ExtensionId::new, |e| e.id);
    let root = root(app);
    let staging = root.join(format!("{}{STAGING_SUFFIX}", ExtensionId::new()));
    let version_parent = root.join(extension_id.to_string());

    let (unpacked, dir, mozilla_signed) = task::spawn_blocking(move || {
        let result = (|| {
            let (archive, key) = match kind {
                Kind::Crx => {
                    let parsed = crx::parse(&bytes)?;

                    (Archive::Zip(parsed.zip), parsed.public_key)
                }
                Kind::Zip => (Archive::Zip(&bytes), None),
                Kind::Tar => (Archive::Tar(&bytes), None),
            };
            let unpacked = unpack(archive, &staging, key.as_deref())?;
            let dir = version_parent.join(&unpacked.version);

            fs::create_dir_all(&version_parent).map_err(internal)?;
            // The same source and version is the same content; a browser may be loading the old copy.
            if dir.exists() {
                fs::remove_dir_all(&staging).map_err(internal)?;
            } else {
                fs::rename(&staging, &dir).map_err(internal)?;
            }

            let mozilla_signed = dir.join(MOZILLA_SIGNATURE).exists();

            Ok((unpacked, dir, mozilla_signed))
        })();
        if result.is_err() {
            fs::remove_dir_all(&staging).ok();
        }

        result
    })
    .await
    .map_err(internal)??;

    // The package decides, not how it was uploaded: a Firefox add-on names gecko in its manifest
    // or carries Mozilla's signature.
    let engine = engine.unwrap_or(match source {
        ExtensionSource::Amo => Engine::Camoufox,
        ExtensionSource::Cws => Engine::Chromium,
        ExtensionSource::Local if is_gecko(&unpacked.manifest) || mozilla_signed => {
            Engine::Camoufox
        }
        ExtensionSource::Local => Engine::Chromium,
    });
    let row = Extension {
        id: extension_id,
        engine,
        source,
        source_ref,
        name: unpacked.name.clone(),
        versions: Vec::new(),
    };
    let version = ExtensionVersion {
        version: unpacked.version,
        path: dir,
        size_bytes: unpacked.size_bytes,
        fetched_at: now_ms(),
        last_referenced_at: None,
    };

    app.store
        .run(move |c| {
            if existing.is_none() {
                extension::insert(c, &row)?;
            }
            extension::add_version(c, &row.id, &version)?;
            extension::set_name(c, &row.id, &row.name)?;

            extension::get(c, &row.id)
        })
        .await
}

fn newest(extension: &Extension) -> Option<&ExtensionVersion> {
    extension.versions.first()
}

pub async fn update(
    app: &Arc<App>,
    extension_id: &ExtensionId,
) -> Result<(Option<String>, String), Error> {
    let id = *extension_id;
    let found = app.store.run(move |c| extension::get(c, &id)).await?;
    let from = newest(&found).map(|v| v.version.clone());
    let current = from.clone().unwrap_or_else(|| "0".to_string());

    let (latest, bytes) = match found.source {
        ExtensionSource::Local => {
            return Err(invalid(
                "extension_id",
                "local extensions cannot be updated; add the new file",
            ));
        }
        ExtensionSource::Cws => {
            let prodversion = prodversion(app).await?;
            let url = cws_url(
                app,
                &format!(
                    "response=updatecheck&prodversion={prodversion}&acceptformat=crx2,crx3&x=id%3D{}%26v%3D{current}%26uc",
                    found.source_ref
                ),
            );
            let item = format!("extension {}", found.source_ref);
            let xml = fetch(app, &url, UPDATECHECK_MAX_BYTES, CWS_SITE, &item).await?;
            let Some((version, codebase)) = crx::scan_updatecheck(&String::from_utf8_lossy(&xml))
            else {
                return Ok((from, current));
            };
            if compare_versions(&version, &current) != std::cmp::Ordering::Greater {
                return Ok((from, current));
            }

            let bytes = fetch(app, &codebase, MAX_EXTENSION_BYTES, CWS_SITE, &item).await?;

            (version, bytes)
        }
        ExtensionSource::Amo => {
            let meta = amo_meta(app, &found.source_ref).await?;
            if compare_versions(&meta.version, &current) != std::cmp::Ordering::Greater {
                return Ok((from, current));
            }

            let bytes = fetch_xpi(app, &meta, &found.source_ref).await?;

            (meta.version.clone(), bytes)
        }
    };

    let updated = install(
        app,
        found.source,
        found.source_ref,
        bytes,
        Some(found.engine),
    )
    .await?;
    let to = newest(&updated).map_or(latest, |v| v.version.clone());
    app.events
        .publish(EventData::ExtensionUpdated {
            extension_id: *extension_id,
            from: from.clone(),
            to: to.clone(),
        })
        .await?;

    Ok((from, to))
}

// Refused while a running browser has its files loaded or a Profile has it enabled, as a Build is:
// removing it would pull the files from under the browser, or drop it from Profiles unannounced.
// A Template only hands it to Profiles it creates, so its link goes with the extension (cascade).
pub async fn remove(app: &Arc<App>, extension_id: &ExtensionId) -> Result<Extension, Error> {
    let id = *extension_id;
    let root = root(app);
    let loaded_dir = root.join(id.to_string());
    let loaded = app
        .instances
        .loaded_extensions()
        .iter()
        .any(|path| path.starts_with(&loaded_dir));
    let removed = app
        .store
        .run(move |c| {
            let found = extension::get(c, &id)?;
            let mut users: Vec<String> = profile_extension::profiles_using(c, &id)?
                .into_iter()
                .map(|(_, name)| format!("profile {name}"))
                .collect();
            if loaded {
                users.push(LOADED_BY_INSTANCE.to_owned());
            }
            if !users.is_empty() {
                return Err(Error::Conflict {
                    reason: ConflictReason::Referenced,
                    detail: Some(users.join(REFERENCE_SEPARATOR)),
                });
            }
            extension::delete(c, &id)?;

            Ok(found)
        })
        .await?;
    let dir = root.join(extension_id.to_string());
    task::spawn_blocking(move || remove_under(&root, &dir))
        .await
        .map_err(internal)??;

    Ok(removed)
}

// A path built from database rows or from the browser is a data-loss and data-leak path: only
// something that really sits under `root` is used. None when the path does not exist.
pub fn contained(root: &Path, path: &Path) -> Result<Option<PathBuf>, Error> {
    let Ok(canonical) = path.canonicalize() else {
        return Ok(None);
    };
    let root = root.canonicalize().map_err(internal)?;
    if !canonical.starts_with(&root) || canonical == root {
        return Err(internal(format!(
            "{} is outside {}",
            canonical.display(),
            root.display()
        )));
    }

    Ok(Some(canonical))
}

pub fn remove_under(root: &Path, path: &Path) -> Result<(), Error> {
    let Some(canonical) = contained(root, path)? else {
        return Ok(());
    };
    let removed = if canonical.is_dir() {
        fs::remove_dir_all(canonical)
    } else {
        fs::remove_file(canonical)
    };

    removed.map_err(internal)
}

// An install killed midway leaves its staging directory; no install outlives the process.
pub fn clear_staging(data_dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(data_dir.join(EXTENSIONS_DIR)) else {
        return 0;
    };

    entries
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with(STAGING_SUFFIX)
        })
        .filter(|entry| fs::remove_dir_all(entry.path()).is_ok())
        .count()
}

fn check_engine(extensions: &[Extension], engine: Engine) -> Result<(), Error> {
    match extensions.iter().find(|e| e.engine != engine) {
        Some(other) => Err(invalid(
            "extensions",
            &format!(
                "{} is a {engine} extension; remove it or choose a {engine} browser version",
                other.name,
                engine = engine_name(other.engine),
            ),
        )),
        None => Ok(()),
    }
}

pub fn check_enable(extensions: &[Extension], build: &Build) -> Result<(), Error> {
    if extensions.is_empty() {
        return Ok(());
    }

    check_engine(extensions, build.engine)?;
    if !build.capabilities.contains(&Capability::UnpackedExtensions) {
        return Err(Error::EngineUnsupported {
            capability: Capability::UnpackedExtensions,
        });
    }

    Ok(())
}

// E2a for a list about to be saved on a Profile or Template. The engine follows from the build
// choice even when nothing of it is installed; capabilities need a Build and are re-checked at launch.
pub fn check_list(
    c: &Connection,
    list: &[ProfileExtension],
    settings: &ProfileSettings,
) -> Result<(), Error> {
    if list.is_empty() {
        return Ok(());
    }
    let extensions = list
        .iter()
        .map(|entry| extension::get(c, &entry.extension_id))
        .collect::<Result<Vec<_>, _>>()?;

    let engine = match &settings.build {
        BuildChoice::Channel(channel) => Some(engine_of(channel.source)),
        BuildChoice::BuildId(build_id) => match build::get(c, build_id) {
            Ok(found) => Some(found.engine),
            Err(Error::NotFound { .. }) => None,
            Err(error) => return Err(error),
        },
    };
    if let Some(engine) = engine {
        check_engine(&extensions, engine)?;
    }

    match resolve_build(c, &settings.build) {
        Ok(build) => check_enable(&extensions, &build),
        Err(Error::BuildMissing { .. }) => Ok(()),
        Err(error) => Err(error),
    }
}

// Each update runs in its own task: a launch that stops waiting at the budget leaves the download
// to finish in the background instead of throwing it away.
async fn auto_update(app: &Arc<App>, ids: Vec<ExtensionId>) {
    let updates = join_all(ids.iter().map(|id| {
        let (app, id) = (Arc::clone(app), *id);

        tokio::spawn(async move { update(&app, &id).await })
    }));

    match time::timeout(UPDATE_BUDGET, updates).await {
        Ok(results) => {
            for (id, result) in ids.iter().zip(results) {
                let failed = match result {
                    Ok(Ok(_)) => continue,
                    Ok(Err(error)) => format!("{error:#}"),
                    Err(error) => error.to_string(),
                };
                warn!(extension_id = %id, error = failed, "extension update failed; using the installed version");
            }
        }
        Err(_) => {
            warn!("extension updates exceeded the launch budget; they finish in the background")
        }
    }
}

// The directories to load and what they are, for the Instance's record.
pub async fn launch_paths(
    app: &Arc<App>,
    profile: &Profile,
    build: &Build,
) -> Result<(Vec<PathBuf>, Vec<InstanceExtension>), Error> {
    let profile_id = profile.id;
    let enabled = app
        .store
        .run(move |c| profile_extension::get(c, &profile_id))
        .await?;
    if enabled.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let load = |app: Arc<App>, enabled: Vec<ProfileExtension>| async move {
        app.store
            .run(move |c| {
                enabled
                    .iter()
                    .map(|entry| extension::get(c, &entry.extension_id))
                    .collect::<Result<Vec<_>, _>>()
            })
            .await
    };
    let extensions = load(Arc::clone(app), enabled.clone()).await?;
    check_enable(&extensions, build)?;

    let stale: Vec<ExtensionId> = enabled
        .iter()
        .zip(&extensions)
        .filter(|(entry, e)| entry.auto_update && e.source != ExtensionSource::Local)
        .map(|(_, e)| e.id)
        .collect();
    let extensions = if stale.is_empty() {
        extensions
    } else {
        auto_update(app, stale).await;

        load(Arc::clone(app), enabled.clone()).await?
    };

    let mut paths = Vec::new();
    let mut loaded = Vec::new();
    let mut used = Vec::new();
    for (entry, e) in enabled.iter().zip(&extensions) {
        let version = match &entry.pinned_version {
            Some(pinned) => e
                .versions
                .iter()
                .find(|v| &v.version == pinned)
                .ok_or_else(|| {
                    invalid(
                        "extensions",
                        &format!("{} {pinned} is not in the library", e.name),
                    )
                })?,
            None => newest(e).ok_or_else(|| {
                invalid(
                    "extensions",
                    &format!("{} has no files in the library", e.name),
                )
            })?,
        };
        paths.push(version.path.clone());
        loaded.push(InstanceExtension {
            id: e.id,
            name: e.name.clone(),
            version: version.version.clone(),
        });
        used.push((e.id, version.version.clone()));
    }
    app.store
        .run(move |c| extension::touch_versions(c, &used, now_ms()))
        .await?;

    Ok((paths, loaded))
}

pub async fn all(app: &Arc<App>) -> Result<Vec<Extension>, Error> {
    app.store
        .run(|c| {
            let mut found = Vec::new();
            let mut cursor = None;
            loop {
                let (page, next) = extension::list(c, &Page::new(Some(PAGE_LIMIT), cursor))?;
                found.extend(page);
                match next {
                    Some(next) => cursor = Some(next),
                    None => return Ok(found),
                }
            }
        })
        .await
}
