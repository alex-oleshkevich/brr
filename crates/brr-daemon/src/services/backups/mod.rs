use std::{
    collections::HashMap,
    fs::{self, DirBuilder},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use brr_core::{
    Backup, BackupId, BackupSchedule, ConflictReason, Error, EventData, ExtensionSource,
    LoginCheck, LoginCheckId, LoginStatus, Profile, ProfileExtension, ProfileId, ProfileKind,
    ProfileSettings, Secret, now_ms,
};
use brr_storage::detached;
use brr_store::{backup, extension, instance, login_check, profile, profile_extension};
use brr_vault::archive::{read_archive, write_archive};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::{
    select,
    sync::broadcast::{Receiver, error::RecvError},
    time,
};
use tracing::warn;

use crate::{
    app::App,
    internal,
    services::{
        events::Published,
        profiles::{self, DEFAULT_BACKUP_KEEP, MAX_NAME_LEN, validate_name},
        proxies, quota,
    },
};

#[cfg(test)]
mod tests;

pub const PASSPHRASE_SECRET: &str = "backup.passphrase";

pub const INLINE_ARCHIVE_MAX: u64 = 512 * 1024 * 1024;

const MANIFEST_FORMAT: u32 = 1;
// The schedule is rechecked against the wall clock this often, so a suspended machine runs a
// missed backup soon after waking instead of a whole sleep late.
const SLEEP_CHUNK: Duration = Duration::from_secs(3600);
const TMP_DIR: &str = "tmp";
const PRIVATE_DIR_MODE: u32 = 0o700;
const ARCHIVE_SUFFIX: &str = "tar.zst.age";
// Caches rebuild themselves; the Singleton* files are P1b's launch locks.
const EXCLUDE: &[&str] = &[
    "Cache",
    "Code Cache",
    "GPUCache",
    "cache2",
    "SingletonLock",
    "SingletonSocket",
    "SingletonCookie",
];
const MISSING_PASSPHRASE: &str = "set a backup passphrase first (settings set backup.passphrase)";
const PROFILE_EVENTS: [&str; 3] = ["profile.created", "profile.updated", "profile.deleted"];

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: u32,
    name: String,
    settings: ProfileSettings,
    notes: String,
    tags: Vec<String>,
    pinned: bool,
    disk_quota_bytes: Option<u64>,
    extensions: Vec<ExtensionRef>,
    login_checks: Vec<LoginCheckRef>,
}

#[derive(Serialize, Deserialize)]
struct ExtensionRef {
    source: ExtensionSource,
    source_ref: String,
    pinned_version: Option<String>,
    auto_update: bool,
}

// No credential id: Credentials never leave the machine they were created on.
#[derive(Serialize, Deserialize)]
struct LoginCheckRef {
    site: String,
    url: String,
    expect_selector: Option<String>,
    expect_text: Option<String>,
    auto_relogin: bool,
}

#[derive(Serialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct MissingExtension {
    pub source: ExtensionSource,
    pub source_ref: String,
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct Imported {
    pub profile: Profile,
    pub missing_extensions: Vec<MissingExtension>,
}

// Profiles being archived: launch refuses them, so the archive never holds a torn database.
#[derive(Default)]
pub struct Exports(Arc<Mutex<HashMap<ProfileId, u32>>>);

pub struct ExportGuard {
    held: Arc<Mutex<HashMap<ProfileId, u32>>>,
    profile_id: ProfileId,
}

impl Exports {
    fn begin(&self, profile_id: ProfileId) -> ExportGuard {
        *self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(profile_id)
            .or_default() += 1;

        ExportGuard {
            held: Arc::clone(&self.0),
            profile_id,
        }
    }

    pub fn is_exporting(&self, profile_id: &ProfileId) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(profile_id)
    }
}

impl Drop for ExportGuard {
    fn drop(&mut self) {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = held.get_mut(&self.profile_id) {
            *count -= 1;
            if *count == 0 {
                held.remove(&self.profile_id);
            }
        }
    }
}

// A passphrase read from a file ends with the editor's newline.
pub fn passphrase(text: &str) -> Result<Secret, Error> {
    let trimmed = text.strip_suffix('\n').unwrap_or(text);
    let trimmed = trimmed.strip_suffix('\r').unwrap_or(trimmed);
    if trimmed.is_empty() {
        return Err(Error::InvalidArgument {
            field: "passphrase".to_string(),
            message: "must not be empty".to_string(),
        });
    }

    Ok(Secret::new(trimmed))
}

// A path under data_dir/tmp that is removed on every exit path, file or directory alike.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(app: &App, name: &str) -> Result<Scratch, Error> {
        let dir = app.paths.data_dir.join(TMP_DIR);
        private_dir(&dir)?;

        Ok(Scratch(dir.join(name)))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

// Nothing in the scratch dir outlives the process: a crash or a kill leaves its files behind.
pub fn clear_scratch(data_dir: &Path) -> bool {
    fs::remove_dir_all(data_dir.join(TMP_DIR)).is_ok()
}

fn private_dir(dir: &Path) -> Result<(), Error> {
    DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(dir)
        .map_err(internal)
}

pub fn archive_filename(profile: &Profile) -> String {
    format!("{}.{ARCHIVE_SUFFIX}", profile.name)
}

pub async fn export_to_file(
    app: &Arc<App>,
    profile: &Profile,
    passphrase: Secret,
) -> Result<(Scratch, u64), Error> {
    let (app, profile) = (Arc::clone(app), profile.clone());

    detached(async move { archive_profile(&app, &profile, passphrase).await }).await
}

async fn archive_profile(
    app: &Arc<App>,
    profile: &Profile,
    passphrase: Secret,
) -> Result<(Scratch, u64), Error> {
    let data_dir = profile
        .data_dir
        .clone()
        .filter(|_| profile.kind == ProfileKind::Persistent)
        .ok_or_else(|| Error::InvalidArgument {
            field: "profile".to_string(),
            message: "only Persistent profiles have data to export".to_string(),
        })?;

    // Taken before the running check: a launch either sees the guard or is seen by the check.
    let _exporting = app.exports.begin(profile.id);
    let exported = profile.clone();
    let manifest = app
        .store
        .run(move |c| {
            if instance::active_for_profile(c, &exported.id)?.is_some() {
                return Err(Error::Conflict {
                    reason: ConflictReason::ProfileRunning,
                    detail: None,
                });
            }

            let extensions = profile_extension::get(c, &exported.id)?
                .into_iter()
                .map(|enabled| {
                    let library = extension::get(c, &enabled.extension_id)?;

                    Ok(ExtensionRef {
                        source: library.source,
                        source_ref: library.source_ref,
                        pinned_version: enabled.pinned_version,
                        auto_update: enabled.auto_update,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            let login_checks = login_check::for_profile(c, &exported.id)?
                .into_iter()
                .map(|check| LoginCheckRef {
                    site: check.site,
                    url: check.url,
                    expect_selector: check.expect_selector,
                    expect_text: check.expect_text,
                    auto_relogin: check.auto_relogin,
                })
                .collect();

            serde_json::to_vec(&Manifest {
                format: MANIFEST_FORMAT,
                name: exported.name,
                settings: exported.settings,
                notes: exported.notes,
                tags: exported.tags,
                pinned: exported.pinned,
                disk_quota_bytes: exported.disk_quota_bytes,
                extensions,
                login_checks,
            })
            .map_err(internal)
        })
        .await?;

    let out = Scratch::new(app, &format!("export-{}", BackupId::new()))?;
    quota::check_disk_floor(app)?;
    let target = out.path().to_owned();
    let size_bytes = tokio::task::spawn_blocking(move || {
        write_archive(&data_dir, &manifest, EXCLUDE, &passphrase, &target)
    })
    .await
    .map_err(internal)??;

    Ok((out, size_bytes))
}

// Takes the archive file so it is removed only after the import has finished, even when the
// caller is gone.
pub async fn import_from_file(
    app: &Arc<App>,
    archive: Scratch,
    passphrase: Secret,
    name: Option<String>,
) -> Result<Imported, Error> {
    if let Some(requested) = &name {
        validate_name(requested, "name")?;
    }
    if archive.path().metadata().map_err(internal)?.len() == 0 {
        return Err(Error::InvalidArgument {
            field: "file".to_string(),
            message: "the archive is empty".to_string(),
        });
    }
    let app = Arc::clone(app);

    detached(async move { import_archive(&app, &archive, passphrase, name).await }).await
}

async fn import_archive(
    app: &Arc<App>,
    archive: &Scratch,
    passphrase: Secret,
    name: Option<String>,
) -> Result<Imported, Error> {
    quota::check_disk_floor(app)?;
    // Extraction may use what is free above the floor, and no more.
    let budget = quota::free_bytes(&app.paths.data_dir)?.saturating_sub(quota::floor_bytes(app));

    // The new id names both the unpack dir and the final folder; nothing in the archive does.
    let profile_id = ProfileId::new();
    let unpacked = Scratch::new(app, &format!("import-{profile_id}"))?;
    private_dir(unpacked.path())?;

    let (archive, dest) = (archive.path().to_owned(), unpacked.path().to_owned());
    let manifest =
        tokio::task::spawn_blocking(move || read_archive(&archive, &passphrase, &dest, budget))
            .await
            .map_err(internal)??;
    let manifest: Manifest = serde_json::from_slice(&manifest).map_err(|_| invalid_file())?;
    if manifest.format != MANIFEST_FORMAT {
        return Err(invalid_file());
    }
    manifest.settings.validate()?;
    // The manifest is untrusted too: its name goes through the rules every other path enforces.
    let base = name.clone().unwrap_or_else(|| manifest.name.clone());
    validate_name(&base, if name.is_some() { "name" } else { "file" })?;

    let profiles_dir = app.paths.profiles_dir();
    let source = unpacked.path().to_owned();
    let imported = app
        .store
        .run(move |c| {
            let mut settings = manifest.settings;
            // The proxy may exist only on the machine the archive came from.
            if proxies::ensure_exists(c, settings.proxy.as_ref()).is_err() {
                settings.proxy = None;
            }
            let now = now_ms();
            let target = profiles_dir.join(profile_id.to_string());
            let mut created = Profile {
                id: profile_id,
                name: base.clone(),
                kind: ProfileKind::Persistent,
                template_id: None,
                settings,
                pinned: manifest.pinned,
                notes: manifest.notes,
                tags: manifest.tags,
                data_dir: Some(target.clone()),
                disk_quota_bytes: manifest.disk_quota_bytes,
                backup_schedule: None,
                backup_keep: DEFAULT_BACKUP_KEEP,
                created_at: now,
                updated_at: now,
                capabilities: Vec::new(),
                login_status: None,
                builtin: false,
            };

            let mut suffix = 0;
            loop {
                match profile::insert(c, &created) {
                    Err(Error::Conflict {
                        reason: ConflictReason::NameTaken,
                        ..
                    }) => {
                        suffix += 1;
                        created.name = suffixed_name(&base, suffix);
                    }
                    inserted => break inserted?,
                }
            }

            let attached = (|| {
                for check in manifest.login_checks {
                    login_check::insert(
                        c,
                        &LoginCheck {
                            id: LoginCheckId::new(),
                            profile_id,
                            site: check.site,
                            url: check.url,
                            expect_selector: check.expect_selector,
                            expect_text: check.expect_text,
                            credential_id: None,
                            auto_relogin: check.auto_relogin,
                            last_status: LoginStatus::Unknown,
                            last_checked_at: None,
                        },
                    )?;
                }

                let mut linked = Vec::new();
                let mut missing = Vec::new();
                for wanted in manifest.extensions {
                    match extension::find_by_source(c, wanted.source, &wanted.source_ref)? {
                        Some(found) => linked.push(ProfileExtension {
                            pinned_version: wanted.pinned_version.filter(|pinned| {
                                found.versions.iter().any(|v| &v.version == pinned)
                            }),
                            extension_id: found.id,
                            auto_update: wanted.auto_update,
                        }),
                        None => missing.push(MissingExtension {
                            source: wanted.source,
                            source_ref: wanted.source_ref,
                        }),
                    }
                }
                profile_extension::set(c, &profile_id, &linked)?;

                fs::create_dir_all(&profiles_dir)
                    .and_then(|()| fs::rename(&source, &target))
                    .map_err(internal)?;

                Ok::<_, Error>(missing)
            })();

            match attached {
                Ok(missing_extensions) => Ok(Imported {
                    profile: profiles::with_capabilities(&brr_store::build::all(c)?, created),
                    missing_extensions,
                }),
                Err(error) => {
                    // The cascade removes the login checks and extension links with the row.
                    let _ = profile::delete(c, &profile_id);

                    Err(error)
                }
            }
        })
        .await?;

    app.events
        .publish(EventData::ProfileCreated {
            profile_id,
            name: imported.profile.name.clone(),
        })
        .await?;

    Ok(imported)
}

// The suffix counts toward the name's length limit; names are ASCII, so bytes are characters.
fn suffixed_name(base: &str, suffix: u32) -> String {
    let suffix = format!("-{suffix}");
    let room = MAX_NAME_LEN.saturating_sub(suffix.len());

    format!(
        "{}{suffix}",
        base.get(..room.min(base.len())).unwrap_or(base)
    )
}

fn invalid_file() -> Error {
    Error::InvalidArgument {
        field: "file".to_string(),
        message: "not a brr profile archive".to_string(),
    }
}

// `passphrase` None uses the backup.passphrase secret, as the scheduler does.
pub async fn create(
    app: &Arc<App>,
    profile: &Profile,
    passphrase: Option<Secret>,
) -> Result<Backup, Error> {
    let (app, profile) = (Arc::clone(app), profile.clone());

    detached(async move { create_and_report(&app, &profile, passphrase).await }).await
}

async fn create_and_report(
    app: &Arc<App>,
    profile: &Profile,
    passphrase: Option<Secret>,
) -> Result<Backup, Error> {
    let profile_id = profile.id;
    let result = store_backup(app, profile, passphrase).await;

    match &result {
        Ok(created) => {
            app.events
                .publish(EventData::BackupCompleted {
                    profile_id,
                    backup_id: Some(created.id),
                })
                .await?;
            prune(app, &profile_id, profile.backup_keep).await;
        }
        Err(Error::Conflict {
            reason: ConflictReason::ProfileRunning,
            ..
        }) => {
            app.events
                .publish(EventData::BackupSkipped { profile_id })
                .await?;
        }
        Err(error) => {
            app.events
                .publish(EventData::BackupFailed {
                    profile_id,
                    error: error.to_string(),
                })
                .await?;
        }
    }

    result
}

async fn store_backup(
    app: &Arc<App>,
    profile: &Profile,
    passphrase: Option<Secret>,
) -> Result<Backup, Error> {
    let passphrase = match passphrase {
        Some(given) => given,
        None => stored_passphrase(app)
            .await?
            .ok_or_else(|| Error::InvalidArgument {
                field: PASSPHRASE_SECRET.to_string(),
                message: MISSING_PASSPHRASE.to_string(),
            })?,
    };
    let (archive, _) = archive_profile(app, profile, passphrase).await?;

    let backend = app.storage.backend();
    let backup_id = BackupId::new();
    // The key prefix ties the object to this backend; see `Backend::holds`.
    let key = format!(
        "{}backups/{}/{backup_id}.{ARCHIVE_SUFFIX}",
        backend.key_prefix(),
        profile.id
    );
    let stored = backend.put_owned_file(&key, archive.path()).await?;
    let row = Backup {
        id: backup_id,
        profile_id: profile.id,
        profile_name: profile.name.clone(),
        profile_deleted: false,
        backend: backend.kind().as_str().to_string(),
        location: key,
        size_bytes: stored.size_bytes,
        created_at: now_ms(),
    };

    let inserted = row.clone();
    if let Err(error) = app.store.run(move |c| backup::insert(c, &inserted)).await {
        let _ = backend.delete(&row.location).await;

        return Err(error);
    }

    Ok(row)
}

pub async fn stored_passphrase(app: &App) -> Result<Option<Secret>, Error> {
    app.secrets.get(&app.store, PASSPHRASE_SECRET).await
}

async fn prune(app: &Arc<App>, profile_id: &ProfileId, keep: u32) {
    let id = *profile_id;
    let rows = match app.store.run(move |c| backup::for_profile(c, &id)).await {
        Ok(rows) => rows,
        Err(error) => {
            warn!(%profile_id, error = format!("{error:#}"), "listing backups to prune failed");

            return;
        }
    };

    // A keep of 0 would delete the backup just made.
    for old in rows.into_iter().skip(keep.max(1) as usize) {
        if let Err(error) = remove(app, &old).await {
            warn!(%profile_id, backup_id = %old.id, error = format!("{error:#}"), "pruning an old backup failed");
        }
    }
}

// The object is deleted only through the backend that stored it; after a backend switch the
// same key would name nothing, or another object.
pub async fn remove(app: &Arc<App>, stored: &Backup) -> Result<(), Error> {
    remove_object(app, stored).await?;

    let backup_id = stored.id;
    app.store.run(move |c| backup::delete(c, &backup_id)).await
}

async fn remove_object(app: &Arc<App>, stored: &Backup) -> Result<(), Error> {
    open_stored(app, stored)?.delete(&stored.location).await
}

fn open_stored(app: &Arc<App>, stored: &Backup) -> Result<brr_storage::Backend, Error> {
    let backend = app.storage.backend();
    if backend.holds(&stored.backend, &stored.location) {
        return Ok(backend);
    }

    Err(Error::InvalidArgument {
        field: "backup_id".to_string(),
        message: format!(
            "backup is stored on {}; switch the storage backend back to it first",
            stored.backend
        ),
    })
}

pub async fn restore(
    app: &Arc<App>,
    backup_id: BackupId,
    passphrase: Option<Secret>,
    name: Option<String>,
) -> Result<Imported, Error> {
    let (app, name) = (Arc::clone(app), name);

    detached(async move { restore_backup(&app, backup_id, passphrase, name).await }).await
}

async fn restore_backup(
    app: &Arc<App>,
    backup_id: BackupId,
    passphrase: Option<Secret>,
    name: Option<String>,
) -> Result<Imported, Error> {
    let stored = app.store.run(move |c| backup::get(c, &backup_id)).await?;
    let passphrase = match passphrase {
        Some(given) => given,
        None => stored_passphrase(app)
            .await?
            .ok_or_else(|| Error::InvalidArgument {
                field: "passphrase".to_string(),
                message: "give the passphrase the backup was made with".to_string(),
            })?,
    };
    let backend = open_stored(app, &stored)?;
    quota::check_disk_floor(app)?;

    let file = Scratch::new(app, &format!("restore-{}", BackupId::new()))?;
    backend.get_to_file(&stored.location, file.path()).await?;

    import_from_file(app, file, passphrase, name).await
}

// Next due time per scheduled Profile; the schedule text was validated on write.
async fn scheduled(app: &App, after: i64) -> Vec<(ProfileId, i64)> {
    let all = match profiles::all(&app.store).await {
        Ok(all) => all,
        Err(error) => {
            warn!(
                error = format!("{error:#}"),
                "listing profiles for the backup scheduler failed"
            );

            return Vec::new();
        }
    };

    all.into_iter()
        .filter_map(|found| {
            let schedule: BackupSchedule = found.backup_schedule?.parse().ok()?;

            Some((found.id, schedule.next_after(after)))
        })
        .collect()
}

// The scheduler sleeps toward `next` in chunks, so a suspended machine is noticed soon after waking.
fn wait_for(next: i64, cursor: i64) -> Duration {
    Duration::from_millis(u64::try_from(next - cursor).unwrap_or_default()).min(SLEEP_CHUNK)
}

// The cursor moves by what was slept, and never behind the wall clock: after a wake it stands at the
// fired minute even if the wall clock has not reached it (paused time), so the next tick is a full
// period away.
fn after_sleep(cursor: i64, waited: Duration, wall: i64) -> i64 {
    (cursor + i64::try_from(waited.as_millis()).unwrap_or_default()).max(wall)
}

// The Profiles whose backup falls at the earliest due time.
fn due_at(upcoming: Vec<(ProfileId, i64)>, next: i64) -> Vec<ProfileId> {
    upcoming
        .into_iter()
        .filter(|(_, at)| *at == next)
        .map(|(profile_id, _)| profile_id)
        .collect()
}

// False once the bus is closed.
async fn profiles_changed(events: &mut Receiver<Arc<Published>>) -> bool {
    loop {
        match events.recv().await {
            Ok(published) if PROFILE_EVENTS.contains(&published.event.kind.as_str()) => {
                return true;
            }
            Ok(_) => {}
            Err(RecvError::Lagged(_)) => return true,
            Err(RecvError::Closed) => return false,
        }
    }
}

// Held weakly: a strong reference would keep the daemon lock after the last owner drops the App.
pub fn start_scheduler(app: &Arc<App>) {
    let weak = Arc::downgrade(app);
    let mut events = app.events.subscribe();

    tokio::spawn(async move {
        let mut cursor = now_ms();

        loop {
            cursor = cursor.max(now_ms());
            let Some(app) = weak.upgrade() else { return };
            let upcoming = scheduled(&app, cursor).await;
            drop(app);

            let Some(next) = upcoming.iter().map(|(_, at)| *at).min() else {
                if !profiles_changed(&mut events).await {
                    return;
                }
                continue;
            };

            let wait = wait_for(next, cursor);
            select! {
                () = time::sleep(wait) => {}
                alive = profiles_changed(&mut events) => {
                    if !alive {
                        return;
                    }
                    continue;
                }
            }

            cursor = after_sleep(cursor, wait, now_ms());
            if cursor < next {
                continue;
            }
            for profile_id in due_at(upcoming, next) {
                let Some(app) = weak.upgrade() else { return };
                backup_one(&app, profile_id).await;
            }
        }
    });
}

async fn backup_one(app: &Arc<App>, profile_id: ProfileId) {
    let found = match app.store.run(move |c| profile::get(c, &profile_id)).await {
        Ok(found) => found,
        Err(error) => {
            warn!(%profile_id, error = format!("{error:#}"), "scheduled backup: profile not found");

            return;
        }
    };

    // create has already published the outcome as backup.completed, skipped or failed.
    if let Err(error) = create(app, &found, None).await {
        warn!(%profile_id, error = format!("{error:#}"), "scheduled backup did not complete");
    }
}
