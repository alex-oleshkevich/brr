use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{Arc, Weak},
    time::Duration,
};

use brr_core::{
    Artifact, ArtifactId, ArtifactKind, Error, EventData, MS_PER_DAY, Paths, ProfileId, Settings,
    SweepCounts, now_ms,
};
use brr_store::{artifact, instance, retention};
use schemars::JsonSchema;
use serde::Serialize;
use tokio::{sync::Mutex, task, time};
use tracing::{info, warn};

use crate::{
    app::App,
    internal,
    services::{
        artifacts, backups,
        extensions::{self, remove_under},
        gc::{self, GcItem, GcKind},
        profiles,
        quota::{BYTES_PER_GB, dir_size},
    },
};

#[cfg(test)]
mod tests;

const SWEEP_INTERVAL: Duration = Duration::from_secs(3600);
const FIRST_SWEEP: Duration = Duration::from_secs(60);
const ARTIFACT_PAGE: u32 = 500;
const DELETE_REASON: &str = "retention";
const CUTOFF_PREFIX: &str = "before:";
const KIND_ARTIFACT: &str = "artifact";
const KIND_EPHEMERAL: &str = "ephemeral";
const KIND_DOWNLOADS: &str = "downloads";
const KIND_BUILD: &str = "build";
const KIND_EXTENSION: &str = "extension";
const KIND_EVENTS: &str = "events";
const KIND_CAPTCHA_SOLVES: &str = "captcha_solves";
const NO_PROFILE: &str = "no profile";

static SWEEPING: Mutex<()> = Mutex::const_new(());

#[derive(Serialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct SweepItem {
    pub kind: String,
    pub id: String,
    pub label: String,
    pub bytes: u64,
}

#[derive(Serialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SweepRule {
    ArtifactsAge,
    ArtifactsSize,
    EphemeralLeftovers,
    UnusedBuilds,
    UnusedExtensions,
    EventsAge,
    CaptchaSolvesAge,
}

#[derive(Serialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct SweepGroup {
    pub rule: SweepRule,
    pub items: Vec<SweepItem>,
    pub bytes: u64,
}

#[derive(Serialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct SweepReport {
    pub dry_run: bool,
    pub groups: Vec<SweepGroup>,
    pub bytes: u64,
}

struct Plan {
    groups: Vec<SweepGroup>,
    events: u32,
}

// Held weakly: a strong reference would keep the daemon lock after the last owner drops the App.
pub fn spawn(app: &Arc<App>) {
    tokio::spawn(run(Arc::downgrade(app)));
}

pub async fn plan(app: &Arc<App>, now: i64) -> Result<Vec<SweepGroup>, Error> {
    Ok(plan_with(app, &app.settings(), now).await?.groups)
}

pub async fn sweep(app: &Arc<App>, dry_run: bool) -> Result<SweepReport, Error> {
    sweep_at(app, now_ms(), dry_run).await
}

// The preview and the deletion share one plan, so a real sweep deletes exactly what the preview listed.
pub async fn sweep_at(app: &Arc<App>, now: i64, dry_run: bool) -> Result<SweepReport, Error> {
    let _only_one = SWEEPING.lock().await;
    let settings = app.settings();
    let planned = plan_with(app, &settings, now).await?;

    let (deleted, bytes) = if dry_run {
        (
            counts(&planned),
            planned.groups.iter().map(|g| g.bytes).sum(),
        )
    } else {
        execute(app, &planned.groups).await?
    };
    // A preview changes nothing, so it announces nothing.
    if !dry_run {
        app.events
            .publish(EventData::SweepCompleted {
                dry_run,
                deleted,
                bytes,
            })
            .await?;
    }

    Ok(SweepReport {
        dry_run,
        groups: planned.groups,
        bytes,
    })
}

async fn run(app: Weak<App>) {
    time::sleep(FIRST_SWEEP).await;

    loop {
        let Some(live) = app.upgrade() else {
            return;
        };
        if let Err(error) = sweep(&live, false).await {
            warn!(error = format!("{error:#}"), "retention sweep failed");
        }
        drop(live);

        time::sleep(SWEEP_INTERVAL).await;
    }
}

async fn plan_with(app: &Arc<App>, settings: &Settings, now: i64) -> Result<Plan, Error> {
    let retention = &settings.retention;
    let profiles: HashMap<ProfileId, _> = profiles::all(&app.store)
        .await?
        .into_iter()
        .map(|profile| (profile.id, profile))
        .collect();

    let label = |found: &Artifact| {
        let profile = found
            .profile_id
            .and_then(|id| profiles.get(&id))
            .map_or(NO_PROFILE, |profile| profile.name.as_str());

        format!("{} {} ({profile})", snake(&found.kind), found.filename)
    };
    let item = |found: &Artifact| SweepItem {
        kind: KIND_ARTIFACT.to_owned(),
        id: found.id.to_string(),
        label: label(found),
        bytes: found.size_bytes,
    };

    let is_expired = |found: &Artifact| {
        let days = found
            .profile_id
            .and_then(|id| profiles.get(&id))
            .and_then(|profile| profile.settings.retention.artifacts_max_age_days)
            .unwrap_or(retention.artifacts_max_age_days);

        found.created_at + i64::from(days) * MS_PER_DAY <= now
    };
    let max_bytes = |profile_id: Option<ProfileId>| {
        let gb = profile_id
            .and_then(|id| profiles.get(&id))
            .and_then(|profile| profile.settings.retention.artifacts_max_total_gb)
            .unwrap_or(retention.artifacts_max_total_gb);

        gb * BYTES_PER_GB
    };

    // Rows are read a page at a time, so memory follows what is swept, not how many Artifacts exist.
    // First pass: what expired, and what each owner keeps.
    let mut expired = Vec::new();
    let mut kept_bytes: BTreeMap<Option<ProfileId>, u64> = BTreeMap::new();
    let mut after = None;
    while let Some(page) = artifact_page(app, after).await? {
        after = page.last().map(|last| (last.created_at, last.id));
        for found in &page {
            if is_expired(found) {
                expired.push(item(found));
            } else {
                *kept_bytes.entry(found.profile_id).or_default() += found.size_bytes;
            }
        }
    }

    // Second pass, only for owners over their cap: oldest first, until what is left fits.
    let mut oversize = Vec::new();
    if kept_bytes
        .iter()
        .any(|(profile_id, total)| *total > max_bytes(*profile_id))
    {
        let mut after = None;
        while let Some(page) = artifact_page(app, after).await? {
            after = page.last().map(|last| (last.created_at, last.id));
            for found in page.iter().filter(|found| !is_expired(found)) {
                let total = kept_bytes.entry(found.profile_id).or_default();
                if *total > max_bytes(found.profile_id) {
                    *total = total.saturating_sub(found.size_bytes);
                    oversize.push(item(found));
                }
            }
        }
    }

    let mut groups = Vec::new();
    push(&mut groups, SweepRule::ArtifactsAge, expired);
    push(&mut groups, SweepRule::ArtifactsSize, oversize);
    push(
        &mut groups,
        SweepRule::EphemeralLeftovers,
        leftovers(app).await?,
    );

    let (builds, extensions): (Vec<_>, Vec<_>) = gc::gc_candidates(app, now, retention.unused_days)
        .await?
        .into_iter()
        .partition(|found| found.kind == GcKind::Build);
    push(
        &mut groups,
        SweepRule::UnusedBuilds,
        builds.into_iter().map(gc_item).collect(),
    );
    push(
        &mut groups,
        SweepRule::UnusedExtensions,
        extensions.into_iter().map(gc_item).collect(),
    );

    let cutoff = now - i64::from(retention.events_days) * MS_PER_DAY;
    let (events, solves) = app
        .store
        .run(move |c| {
            Ok((
                retention::events_before(c, cutoff)?,
                retention::captcha_solves_before(c, cutoff)?,
            ))
        })
        .await?;
    let by_age = |kind: &str, count: u32, what: &str| {
        (count > 0).then(|| {
            vec![SweepItem {
                kind: kind.to_owned(),
                id: format!("{CUTOFF_PREFIX}{cutoff}"),
                label: format!("{count} {what} older than {} days", retention.events_days),
                bytes: 0,
            }]
        })
    };
    push(
        &mut groups,
        SweepRule::EventsAge,
        by_age(KIND_EVENTS, events, "events").unwrap_or_default(),
    );
    push(
        &mut groups,
        SweepRule::CaptchaSolvesAge,
        by_age(KIND_CAPTCHA_SOLVES, solves, "captcha solves").unwrap_or_default(),
    );

    Ok(Plan { groups, events })
}

// None once every row has been read; `after` is the last row of the previous page.
async fn artifact_page(
    app: &Arc<App>,
    after: Option<(i64, ArtifactId)>,
) -> Result<Option<Vec<Artifact>>, Error> {
    let page = app
        .store
        .run(move |c| artifact::oldest_page(c, after, ARTIFACT_PAGE))
        .await?;

    Ok((!page.is_empty()).then_some(page))
}

fn push(groups: &mut Vec<SweepGroup>, rule: SweepRule, items: Vec<SweepItem>) {
    if items.is_empty() {
        return;
    }

    groups.push(SweepGroup {
        rule,
        bytes: items.iter().map(|item| item.bytes).sum(),
        items,
    });
}

fn gc_item(found: GcItem) -> SweepItem {
    SweepItem {
        kind: match found.kind {
            GcKind::Build => KIND_BUILD,
            GcKind::Extension => KIND_EXTENSION,
        }
        .to_owned(),
        id: found.id,
        label: found.label,
        bytes: found.bytes,
    }
}

fn gc_found(item: &SweepItem) -> GcItem {
    GcItem {
        kind: if item.kind == KIND_BUILD {
            GcKind::Build
        } else {
            GcKind::Extension
        },
        id: item.id.clone(),
        label: item.label.clone(),
        bytes: item.bytes,
    }
}

// Directories named for Instances that are not running: what a crash or an old version left.
async fn leftovers(app: &Arc<App>) -> Result<Vec<SweepItem>, Error> {
    let active = active_names(app).await?;
    let roots = [
        (KIND_EPHEMERAL, app.paths.ephemeral_dir()),
        (KIND_DOWNLOADS, app.paths.downloads_dir()),
    ];
    let mut found = Vec::new();

    for (kind, root) in roots {
        let listed = root.clone();
        let names = task::spawn_blocking(move || {
            fs::read_dir(listed)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        })
        .await
        .map_err(internal)?;

        for name in names.into_iter().filter(|name| !active.contains(name)) {
            let dir = root.join(&name);
            let label = dir
                .strip_prefix(&app.paths.data_dir)
                .unwrap_or(&dir)
                .display()
                .to_string();
            found.push(SweepItem {
                kind: kind.to_owned(),
                id: name,
                label,
                bytes: dir_size(dir).await,
            });
        }
    }

    Ok(found)
}

async fn active_names(app: &Arc<App>) -> Result<HashSet<String>, Error> {
    let rows = app.store.run(|c| instance::non_final(c)).await?;
    let mut names: HashSet<String> = rows.into_iter().map(|row| row.id.to_string()).collect();
    names.extend(
        app.instances
            .views()
            .into_iter()
            .map(|view| view.instance.id.to_string()),
    );

    Ok(names)
}

async fn execute(app: &Arc<App>, groups: &[SweepGroup]) -> Result<(SweepCounts, u64), Error> {
    let mut deleted = SweepCounts {
        artifacts: 0,
        builds: 0,
        extensions: 0,
        events: 0,
        backups: 0,
    };
    let mut freed = 0;

    for group in groups {
        match group.rule {
            SweepRule::ArtifactsAge | SweepRule::ArtifactsSize => {
                for item in &group.items {
                    match delete_artifact(app, &item.id).await {
                        Ok(()) => {
                            deleted.artifacts += 1;
                            freed += item.bytes;
                        }
                        Err(error) => warn!(
                            item = item.id.as_str(),
                            error = format!("{error:#}"),
                            "artifact not swept"
                        ),
                    }
                }
            }
            SweepRule::EphemeralLeftovers => {
                for item in &group.items {
                    if remove_leftover(app, item).await? {
                        freed += item.bytes;
                    }
                }
            }
            SweepRule::UnusedBuilds | SweepRule::UnusedExtensions => {
                let items: Vec<GcItem> = group.items.iter().map(gc_found).collect();
                let (count, bytes) = gc::gc_delete(app, &items).await?;
                freed += bytes;
                if group.rule == SweepRule::UnusedBuilds {
                    deleted.builds += count;
                } else {
                    deleted.extensions += count;
                }
            }
            SweepRule::EventsAge => {
                let cutoff = cutoff_of(&group.items[0])?;
                deleted.events = app
                    .store
                    .run(move |c| retention::delete_events_before(c, cutoff))
                    .await?;
            }
            SweepRule::CaptchaSolvesAge => {
                let cutoff = cutoff_of(&group.items[0])?;
                app.store
                    .run(move |c| retention::delete_captcha_solves_before(c, cutoff))
                    .await?;
            }
        }
    }

    Ok((deleted, freed))
}

async fn delete_artifact(app: &Arc<App>, id: &str) -> Result<(), Error> {
    let artifact_id: ArtifactId = id.parse()?;
    let found = app
        .store
        .run(move |c| artifact::get(c, &artifact_id))
        .await?;

    artifacts::delete(app, &found, DELETE_REASON).await
}

// A directory is a data-loss path: an Instance that started since the preview keeps its own.
async fn remove_leftover(app: &Arc<App>, item: &SweepItem) -> Result<bool, Error> {
    if active_names(app).await?.contains(&item.id) {
        warn!(
            dir = item.id.as_str(),
            "instance started since the preview; its directory stays"
        );

        return Ok(false);
    }

    let root = if item.kind == KIND_DOWNLOADS {
        app.paths.downloads_dir()
    } else {
        app.paths.ephemeral_dir()
    };
    let dir: PathBuf = root.join(&item.id);
    task::spawn_blocking(move || remove_under(&root, &dir))
        .await
        .map_err(internal)??;

    Ok(true)
}

// Run once at startup, after the daemon lock is held: no other process can be using these files.
pub fn clear_stale(paths: &Paths) {
    let scratch = backups::clear_scratch(&paths.data_dir);
    let staging = extensions::clear_staging(&paths.data_dir);
    let temp = brr_storage::clear_temp(&paths.storage_dir());

    if scratch || staging > 0 || temp > 0 {
        info!(
            scratch,
            staging, temp, "stale files from an earlier run removed"
        );
    }
}

fn cutoff_of(item: &SweepItem) -> Result<i64, Error> {
    item.id
        .strip_prefix(CUTOFF_PREFIX)
        .and_then(|cutoff| cutoff.parse().ok())
        .ok_or_else(|| internal(format!("bad retention cutoff {}", item.id)))
}

fn counts(planned: &Plan) -> SweepCounts {
    let in_rules = |rules: &[SweepRule]| -> u32 {
        planned
            .groups
            .iter()
            .filter(|group| rules.contains(&group.rule))
            .map(|group| u32::try_from(group.items.len()).unwrap_or(u32::MAX))
            .sum()
    };

    SweepCounts {
        artifacts: in_rules(&[SweepRule::ArtifactsAge, SweepRule::ArtifactsSize]),
        builds: in_rules(&[SweepRule::UnusedBuilds]),
        extensions: in_rules(&[SweepRule::UnusedExtensions]),
        events: planned.events,
        backups: 0,
    }
}

fn snake(kind: &ArtifactKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}
