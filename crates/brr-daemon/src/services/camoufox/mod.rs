use std::{
    fs,
    net::IpAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use brr_builds::{
    GEOIP_URL, Progress, download, fonts_bundle_release, install_fonts_bundle, install_geoip,
};
use brr_camoufox::{
    data::{FpgenPin, fpgen_pin},
    fpgen::model::{Model, unpack_model},
    geoip::{Geo, GeoDb},
};
use brr_core::{
    Asset, AssetId, AssetKind, BuildId, Error, EventData, Geolocation, InstanceId, Profile,
    ProfileKind, ProfileSettings, now_ms,
};
use brr_engine::CamoufoxSpec;
use brr_store::asset;
use sha2::{Digest, Sha256};
use tokio::{sync::mpsc, task};
use tracing::{debug, warn};

use crate::{
    app::App,
    internal,
    services::{builds::source_urls, proxies},
};

#[cfg(test)]
mod tests;

const ASSETS_DIR: &str = "assets";
const FONTS_DIR_PREFIX: &str = "fonts-bundle-";
const MODEL_DIR: &str = "fpgen-model";
const GEOIP_DIR: &str = "geoip";
const MODEL_DIR_HASH_CHARS: usize = 12;
const PROGRESS_BUFFER: usize = 16;
const GEOIP_VERSION: &str = "latest";
const GEOIP_MAX_AGE_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const PUBLIC_IP_URL: &str = "https://api.ipify.org";
const PUBLIC_IP_BUDGET: Duration = Duration::from_secs(5);
const PUBLIC_IP_MAX_BYTES: usize = 64;
const PUBLIC_IP_TTL: Duration = Duration::from_secs(10 * 60);
const GEOIP: &str = "geoip";

#[derive(Default)]
pub struct CamoufoxAssets {
    // One asset install at a time: the fonts bundle alone is close to a gigabyte.
    installing: tokio::sync::Mutex<()>,
    model: Mutex<Option<(String, Arc<Model>)>>,
    // With the row's updated_at, so every access can tell when the database has gone stale.
    geodb: Mutex<Option<(i64, Arc<GeoDb>)>>,
    public_ip: Mutex<Option<(Instant, IpAddr)>>,
    refreshing: AtomicBool,
}

pub struct Assets {
    pub fonts_dir: PathBuf,
    pub model: Arc<Model>,
}

fn geoip_unavailable(reason: impl std::fmt::Display) -> Error {
    Error::DependencyMissing {
        name: GEOIP.to_owned(),
        help: reason.to_string(),
    }
}

fn assets_dir(app: &App) -> PathBuf {
    app.paths.data_dir.join(ASSETS_DIR)
}

async fn installed(app: &Arc<App>, kind: AssetKind) -> Result<Vec<Asset>, Error> {
    app.store.run(move |c| asset::by_kind(c, kind)).await
}

async fn record(
    app: &Arc<App>,
    id: AssetId,
    kind: AssetKind,
    version: String,
    path: PathBuf,
) -> Result<Asset, Error> {
    let row = Asset {
        id,
        kind,
        version,
        path,
        updated_at: now_ms(),
    };
    let saved = row.clone();
    app.store.run(move |c| asset::put(c, &saved)).await?;

    Ok(row)
}

fn cached_model(app: &App, pin: &FpgenPin) -> Option<Arc<Model>> {
    let cached = app
        .camoufox
        .model
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();

    cached
        .filter(|(version, _)| *version == pin.sha256)
        .map(|(_, model)| model)
}

// Everything installed and loaded already: no lock, so a download in progress never holds a
// launch back.
async fn found_assets(app: &Arc<App>) -> Result<Option<Assets>, Error> {
    let Some(model) = cached_model(app, fpgen_pin()) else {
        return Ok(None);
    };
    let Some(fonts) = installed(app, AssetKind::FontsBundle).await?.pop() else {
        return Ok(None);
    };
    if installed(app, AssetKind::GeoipDb).await?.is_empty() {
        return Ok(None);
    }

    Ok(Some(Assets {
        fonts_dir: fonts.path,
        model,
    }))
}

// Idempotent; the fonts bundle, the fpgen model and the GeoIP database install with the first
// Camoufox Build, and every launch finds them done.
pub async fn ensure_assets(app: &Arc<App>, build_id: BuildId) -> Result<Assets, Error> {
    if let Some(assets) = found_assets(app).await? {
        return Ok(assets);
    }
    let _installing = app.camoufox.installing.lock().await;

    let fonts_dir = ensure_fonts(app, build_id).await?;
    let model = ensure_model(app).await?;
    ensure_geoip(app).await?;

    Ok(Assets { fonts_dir, model })
}

async fn ensure_fonts(app: &Arc<App>, build_id: BuildId) -> Result<PathBuf, Error> {
    if let Some(found) = installed(app, AssetKind::FontsBundle).await?.pop() {
        return Ok(found.path);
    }

    let release = fonts_bundle_release(&app.http, &source_urls(app)).await?;
    let dir = assets_dir(app).join(format!("{FONTS_DIR_PREFIX}{}", release.version));
    let (progress, mut updates) = mpsc::channel::<Progress>(PROGRESS_BUFFER);
    let forward = async {
        while let Some(update) = updates.recv().await {
            let event = EventData::BuildInstallProgress {
                build_id,
                bytes: update.bytes,
                total: update.total,
                phase: update.phase,
            };
            if let Err(error) = app.events.publish(event).await {
                warn!(%build_id, %error, "publishing install progress failed");
            }
        }
    };
    let (result, ()) = tokio::join!(
        install_fonts_bundle(&app.http, &release, &dir, progress),
        forward
    );
    result?;

    record(
        app,
        AssetId::new(),
        AssetKind::FontsBundle,
        release.version,
        dir.clone(),
    )
    .await?;

    Ok(dir)
}

async fn ensure_model(app: &Arc<App>) -> Result<Arc<Model>, Error> {
    let pin = fpgen_pin();
    if let Some(model) = cached_model(app, pin) {
        return Ok(model);
    }

    let rows = installed(app, AssetKind::FpgenModel).await?;
    let dir = match rows.iter().find(|row| row.version == pin.sha256) {
        Some(found) => found.path.clone(),
        None => {
            let dir = install_model(app, pin).await?;
            drop_old_models(app, rows).await;

            dir
        }
    };
    let model = task::spawn_blocking(move || Model::open(&dir))
        .await
        .map_err(internal)??;
    *app.camoufox
        .model
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some((pin.sha256.clone(), Arc::clone(&model)));

    Ok(model)
}

async fn install_model(app: &Arc<App>, pin: &'static FpgenPin) -> Result<PathBuf, Error> {
    let root = assets_dir(app).join(MODEL_DIR);
    let name = &pin.sha256[..MODEL_DIR_HASH_CHARS];
    let dir = root.join(name);
    let part = root.join(format!("{name}.zip.part"));
    let tmp = root.join(format!("{name}.tmp"));
    fs::create_dir_all(&root).map_err(internal)?;

    download(&app.http, &pin.url, &part, Some(&pin.sha256), None).await?;

    let (unpacked, dest) = (tmp, dir.clone());
    task::spawn_blocking(move || {
        let result = (|| {
            if unpacked.exists() {
                fs::remove_dir_all(&unpacked).map_err(internal)?;
            }
            fs::create_dir_all(&unpacked).map_err(internal)?;
            unpack_model(&part, pin, &unpacked)?;
            if dest.exists() {
                fs::remove_dir_all(&dest).map_err(internal)?;
            }

            fs::rename(&unpacked, &dest).map_err(internal)
        })();
        fs::remove_dir_all(&unpacked).ok();
        fs::remove_file(&part).ok();

        result
    })
    .await
    .map_err(internal)??;
    record(
        app,
        AssetId::new(),
        AssetKind::FpgenModel,
        pin.sha256.clone(),
        dir.clone(),
    )
    .await?;

    Ok(dir)
}

// Only directories this module installed go: a row pointing elsewhere is someone's own copy.
async fn drop_old_models(app: &Arc<App>, old: Vec<Asset>) {
    let root = assets_dir(app);
    for row in old {
        let id = row.id;
        if let Err(error) = app.store.run(move |c| asset::delete(c, &id)).await {
            warn!(%error, "old fpgen model row not removed");

            continue;
        }
        if row.path.starts_with(&root) {
            fs::remove_dir_all(&row.path).ok();
        }
    }
}

async fn ensure_geoip(app: &Arc<App>) -> Result<Asset, Error> {
    if let Some(found) = installed(app, AssetKind::GeoipDb).await?.pop() {
        return Ok(found);
    }

    install_geoip_db(app).await
}

// The new file arrives by rename, so a refresh needs no lock; it keeps the row's id, one
// GeoIP row for the one file.
async fn install_geoip_db(app: &Arc<App>) -> Result<Asset, Error> {
    let dir = assets_dir(app).join(GEOIP_DIR);
    let file = install_geoip(&app.http, GEOIP_URL, &dir).await?;
    let id = installed(app, AssetKind::GeoipDb)
        .await?
        .pop()
        .map_or_else(AssetId::new, |row| row.id);
    let row = record(app, id, AssetKind::GeoipDb, GEOIP_VERSION.to_owned(), file).await?;
    app.camoufox
        .geodb
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();

    Ok(row)
}

fn stale(updated_at: i64) -> bool {
    now_ms() - updated_at > GEOIP_MAX_AGE_MS
}

fn refresh_geoip(app: &Arc<App>) {
    if app.camoufox.refreshing.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = Arc::clone(app);

    tokio::spawn(async move {
        if let Err(error) = install_geoip_db(&app).await {
            warn!(%error, "GeoIP database not refreshed");
        }
        app.camoufox.refreshing.store(false, Ordering::Release);
    });
}

// The database in use stays until a refresh lands; the age is checked on every access.
async fn geodb(app: &Arc<App>) -> Result<Arc<GeoDb>, Error> {
    let cached = app
        .camoufox
        .geodb
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some((updated_at, db)) = cached {
        if stale(updated_at) {
            refresh_geoip(app);
        }

        return Ok(db);
    }

    let found = {
        let _installing = app.camoufox.installing.lock().await;
        ensure_geoip(app).await?
    };
    if stale(found.updated_at) {
        refresh_geoip(app);
    }

    let (updated_at, path) = (found.updated_at, found.path);
    let db = Arc::new(
        task::spawn_blocking(move || GeoDb::open(&path))
            .await
            .map_err(internal)??,
    );
    *app.camoufox
        .geodb
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some((updated_at, Arc::clone(&db)));

    Ok(db)
}

// The one request that leaves the machine for geoip: a plain GET, nothing else sent.
async fn public_ip(app: &Arc<App>) -> Result<IpAddr, Error> {
    let cached = *app
        .camoufox
        .public_ip
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some((at, ip)) = cached
        && at.elapsed() < PUBLIC_IP_TTL
    {
        return Ok(ip);
    }

    let mut response = app
        .http
        .get(PUBLIC_IP_URL)
        .timeout(PUBLIC_IP_BUDGET)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| geoip_unavailable(format!("the exit IP lookup failed: {error}")))?;
    let chunk = response
        .chunk()
        .await
        .map_err(|error| geoip_unavailable(format!("the exit IP lookup failed: {error}")))?
        .unwrap_or_default();
    let body = &chunk[..chunk.len().min(PUBLIC_IP_MAX_BYTES)];
    let ip: IpAddr = String::from_utf8_lossy(body)
        .trim()
        .parse()
        .map_err(|_| geoip_unavailable("the exit IP lookup returned no address"))?;

    debug!(%ip, "exit IP looked up");
    *app.camoufox
        .public_ip
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some((Instant::now(), ip));

    Ok(ip)
}

// None when the Profile leaves geoip off. The exit IP is the launch's proxy's, else this
// machine's own; a lookup that cannot work fails the launch with the reason.
pub async fn launch_geo(
    app: &Arc<App>,
    instance_id: &InstanceId,
    profile: &Profile,
) -> Result<Option<Geo>, Error> {
    if !profile.settings.geoip {
        return Ok(None);
    }

    let ip = match proxies::exit_ip(app, instance_id).await? {
        Some(text) => text
            .parse()
            .map_err(|_| geoip_unavailable("the proxy's exit IP is not an address"))?,
        None if profile.settings.proxy.is_some() => {
            return Err(geoip_unavailable("the proxy has no known exit IP"));
        }
        None => public_ip(app).await?,
    };
    let db = geodb(app).await.map_err(geoip_unavailable)?;

    db.lookup(ip).map(Some).map_err(geoip_unavailable)
}

// What the Profile left unset follows the exit IP.
pub fn apply_geo(settings: &mut ProfileSettings, geo: &Geo) {
    settings
        .timezone
        .get_or_insert_with(|| geo.timezone.clone());
    settings.locale.get_or_insert_with(|| geo.locale.clone());
    settings.geolocation.get_or_insert(Geolocation {
        latitude: geo.latitude,
        longitude: geo.longitude,
        accuracy: None,
    });
}

// A Persistent Profile keeps one identity: the same seed, so the same draw while the model and
// the fingerprint settings stay the same. An Ephemeral one draws afresh each launch.
fn seed(profile: &Profile) -> Option<u64> {
    if profile.kind != ProfileKind::Persistent {
        return None;
    }
    let digest = Sha256::digest(profile.id.to_string());

    Some(u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]))
}

pub async fn launch_spec(
    app: &Arc<App>,
    profile: &Profile,
    build_id: BuildId,
    geo: Option<Geo>,
    addons: Vec<PathBuf>,
) -> Result<CamoufoxSpec, Error> {
    let assets = ensure_assets(app, build_id).await?;

    Ok(CamoufoxSpec {
        fonts_dir: assets.fonts_dir,
        model: assets.model,
        geo,
        seed: seed(profile),
        addons,
    })
}
