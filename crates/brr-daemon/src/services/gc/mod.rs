use std::{collections::HashSet, path::PathBuf, sync::Arc};

use brr_core::{Build, BuildSource, BuildState, ConflictReason, Error, ExtensionId, MS_PER_DAY};
use brr_store::{build, extension, profile_extension, template_extension};
use tokio::task;
use tracing::warn;

use crate::{
    app::App,
    internal,
    services::{
        builds,
        extensions::{self, remove_under},
        quota,
    },
};

const VERSION_SEPARATOR: char = '@';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcKind {
    Build,
    Extension,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcItem {
    pub kind: GcKind,
    pub id: String,
    pub label: String,
    pub bytes: u64,
}

fn build_dir(app: &App, build: &Build) -> PathBuf {
    app.paths
        .builds_dir()
        .join(build.source.as_str())
        .join(&build.version)
}

// The versions Profiles and Templates use: the pinned one, else the newest.
fn referenced_versions(
    c: &brr_store::Connection,
    library: &[brr_core::Extension],
    loaded: &[PathBuf],
) -> Result<HashSet<(ExtensionId, String)>, Error> {
    // <extension_id>/<version>: what a running browser loaded stays, whatever the Profile says now.
    let mut used: HashSet<(ExtensionId, String)> = loaded
        .iter()
        .filter_map(|path| {
            let version = path.file_name()?.to_str()?.to_string();
            let id = path.parent()?.file_name()?.to_str()?.parse().ok()?;

            Some((id, version))
        })
        .collect();

    let profiles = profile_extension::all(c)?
        .into_iter()
        .map(|(_, entry)| entry);
    for entry in profiles.chain(template_extension::all(c)?) {
        let version = entry.pinned_version.or_else(|| {
            library
                .iter()
                .find(|e| e.id == entry.extension_id)
                .and_then(|e| e.versions.first())
                .map(|v| v.version.clone())
        });
        if let Some(version) = version {
            used.insert((entry.extension_id, version));
        }
    }

    Ok(used)
}

pub async fn gc_candidates(
    app: &Arc<App>,
    now: i64,
    unused_days: u32,
) -> Result<Vec<GcItem>, Error> {
    let cutoff = now - i64::from(unused_days) * MS_PER_DAY;
    let library = extensions::all(app).await?;
    let loaded = app.instances.loaded_extensions();
    let (stale_builds, stale_versions) = app
        .store
        .run(move |c| {
            let all = build::all(c)?;
            let views = builds::views(c, all)?;
            let referenced: Vec<_> = views
                .iter()
                .filter(|v| !v.referenced_by.is_empty())
                .map(|v| v.build.id)
                .collect();
            build::touch_referenced(c, &referenced, now)?;

            let used = referenced_versions(c, &library, &loaded)?;
            let touched: Vec<_> = used.iter().cloned().collect();
            extension::touch_versions(c, &touched, now)?;

            let stale_builds: Vec<Build> = views
                .into_iter()
                .filter(|v| v.referenced_by.is_empty())
                .map(|v| v.build)
                .filter(|b| b.state == BuildState::Installed && b.source != BuildSource::System)
                .filter(|b| {
                    b.last_referenced_at
                        .or(b.installed_at)
                        .is_some_and(|at| at < cutoff)
                })
                .collect();
            let stale_versions: Vec<_> = library
                .iter()
                .flat_map(|e| {
                    // The newest version keeps the library entry usable.
                    e.versions.iter().skip(1).map(move |v| (e, v))
                })
                .filter(|(e, v)| !used.contains(&(e.id, v.version.clone())))
                .filter(|(_, v)| v.last_referenced_at.unwrap_or(v.fetched_at) < cutoff)
                .map(|(e, v)| (e.id, e.name.clone(), v.clone()))
                .collect();

            Ok((stale_builds, stale_versions))
        })
        .await?;

    let mut items = Vec::new();
    for found in stale_builds {
        let dir = build_dir(app, &found);
        let bytes = quota::dir_size(dir).await;
        items.push(GcItem {
            kind: GcKind::Build,
            id: found.id.to_string(),
            label: format!("{} {}", found.name, found.version),
            bytes,
        });
    }
    for (id, name, version) in stale_versions {
        let dir = version.path.clone();
        let bytes = quota::dir_size(dir).await;
        items.push(GcItem {
            kind: GcKind::Extension,
            id: format!("{id}{VERSION_SEPARATOR}{}", version.version),
            label: format!("{name} {}", version.version),
            bytes,
        });
    }

    Ok(items)
}

// Each item is checked again: one that was pinned or enabled since the preview is skipped.
// Returns how many items went and the bytes they held.
pub async fn gc_delete(app: &Arc<App>, items: &[GcItem]) -> Result<(u32, u64), Error> {
    let mut deleted = 0;
    let mut freed = 0;

    for item in items {
        let outcome = match item.kind {
            GcKind::Build => delete_build(app, item).await,
            GcKind::Extension => delete_version(app, item).await,
        };
        match outcome {
            Ok(true) => {
                deleted += 1;
                freed += item.bytes;
            }
            Ok(false) => {}
            Err(error) => warn!(
                item = item.id.as_str(),
                error = format!("{error:#}"),
                "gc skipped an item"
            ),
        }
    }

    Ok((deleted, freed))
}

async fn delete_build(app: &Arc<App>, item: &GcItem) -> Result<bool, Error> {
    let build_id = item.id.parse().map_err(|_| internal("bad build id"))?;

    match builds::remove(app, &build_id).await {
        Ok(_) => Ok(true),
        Err(Error::Conflict {
            reason: ConflictReason::Referenced,
            ..
        }) => Ok(false),
        Err(error) => Err(error),
    }
}

async fn delete_version(app: &Arc<App>, item: &GcItem) -> Result<bool, Error> {
    let (id, version) = item
        .id
        .split_once(VERSION_SEPARATOR)
        .ok_or_else(|| internal("bad extension item id"))?;
    let extension_id: ExtensionId = id.parse().map_err(|_| internal("bad extension id"))?;
    let version = version.to_string();
    let library = extensions::all(app).await?;
    let loaded = app.instances.loaded_extensions();
    let root = extensions::root(app);

    let removed = {
        let version = version.clone();
        app.store
            .run(move |c| {
                let used = referenced_versions(c, &library, &loaded)?;
                let found = extension::get(c, &extension_id)?;
                let Some(entry) = found.versions.iter().find(|v| v.version == version) else {
                    return Ok(None);
                };
                let newest = found.versions.first().is_some_and(|v| v.version == version);
                if newest || used.contains(&(extension_id, version.clone())) {
                    return Ok(None);
                }
                extension::delete_version(c, &extension_id, &version)?;

                Ok(Some(entry.path.clone()))
            })
            .await?
    };
    let Some(dir) = removed else {
        return Ok(false);
    };
    task::spawn_blocking(move || remove_under(&root, &dir))
        .await
        .map_err(internal)??;

    Ok(true)
}
