use std::{
    cmp::Ordering,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use brr_builds::{
    Detected, Progress, Release, SourceUrls, Want, capabilities, detect_system,
    install as install_release, read_compat, resolve,
};
use brr_core::{
    Build, BuildChoice, BuildId, BuildRefKind, BuildReference, BuildSource, BuildState, Channel,
    ConflictReason, Engine, Error, EventData, TemplateId, compare_versions, now_ms,
};
use brr_store::{Connection, build, instance, profile, template};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::{sync::mpsc, task};
use tracing::{error, warn};

use crate::{
    app::App,
    internal,
    services::{
        camoufox,
        compat::{self, Gate},
    },
};

pub mod channels;

const PROGRESS_BUFFER: usize = 16;
const REFERENCE_SEPARATOR: &str = ", ";
const TMP_SUFFIX: &str = "tmp";
const PART_SUFFIX: &str = ".part";

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct BuildView {
    #[serde(flatten)]
    pub build: Build,
    pub referenced_by: Vec<BuildReference>,
}

pub fn ensure_installable(source: BuildSource) -> Result<(), Error> {
    if source != BuildSource::System {
        return Ok(());
    }

    Err(Error::InvalidArgument {
        field: "source".to_string(),
        message: format!(
            "source {source} cannot be installed here; supported: cft, headless_shell, ungoogled, fingerprint_chromium, brave, edge, camoufox"
        ),
    })
}

pub fn source_urls(app: &App) -> SourceUrls {
    let settings = app.settings();

    SourceUrls {
        cft: app.cft_base_url.clone(),
        github_api: settings.builds.github_api.clone(),
        edge_repo: settings.builds.edge_repo.clone(),
    }
}

pub fn engine_of(source: BuildSource) -> Engine {
    match source {
        BuildSource::Camoufox => Engine::Camoufox,
        _ => Engine::Chromium,
    }
}

fn display_name(source: BuildSource) -> &'static str {
    match source {
        BuildSource::Cft => "Chrome for Testing",
        BuildSource::HeadlessShell => "chrome-headless-shell",
        BuildSource::Ungoogled => "ungoogled-chromium",
        BuildSource::FingerprintChromium => "fingerprint-chromium",
        BuildSource::Brave => "Brave",
        BuildSource::Edge => "Microsoft Edge",
        BuildSource::Camoufox => "Camoufox",
        BuildSource::System => "System browser",
    }
}

pub async fn detect(app: &Arc<App>) -> Result<Vec<Build>, Error> {
    let detected = detect_system(&app.path_env).await;

    app.store
        .run(move |c| {
            for found in detected {
                register(c, found)?;
            }

            Ok(build::all(c)?
                .into_iter()
                .filter(|b| b.source == BuildSource::System)
                .collect())
        })
        .await
}

fn register(c: &Connection, found: Detected) -> Result<(), Error> {
    let existing = build::find_by_path(c, &found.path)?;
    let same_version = build::find(c, BuildSource::System, &found.version)?;
    // The same browser found at a better path (Chrome's vendor binary instead of a distro
    // wrapper) moves the Build, so the Profiles that use it follow.
    if existing.is_none()
        && let Some(mut b) = same_version.clone().filter(|b| b.name == found.name)
    {
        b.path = found.path;

        return build::update(c, &b);
    }
    let clashes = same_version
        .as_ref()
        .is_some_and(|b| existing.as_ref().is_none_or(|e| e.id != b.id));
    if clashes {
        warn!(path = ?found.path, version = found.version, "another system build has this version; skipped");

        return Ok(());
    }

    match existing {
        Some(mut b) if b.version != found.version => {
            b.version = found.version;

            build::update(c, &b)
        }
        Some(_) => Ok(()),
        None => build::insert(
            c,
            &Build {
                id: BuildId::new(),
                source: BuildSource::System,
                engine: Engine::Chromium,
                name: found.name,
                version: found.version,
                channel: None,
                path: found.path,
                state: BuildState::Registered,
                sha256: None,
                size_bytes: 0,
                capabilities: capabilities(BuildSource::System),
                installed_at: Some(now_ms()),
                last_referenced_at: None,
            },
        ),
    }
}

pub async fn install(app: &Arc<App>, source: BuildSource, want: Want) -> Result<Build, Error> {
    ensure_installable(source)?;
    let release = resolve(&app.http, &source_urls(app), source, &want).await?;

    // The row state, decided in one transaction, is what keeps a Build to one install at a time.
    let version = release.version.clone();
    let channel = release.channel.clone();
    let path = app
        .paths
        .builds_dir()
        .join(source.as_str())
        .join(&release.version);
    let brr_version = app.version;
    let (row, started, regate) = app
        .store
        .run(move |c| {
            let Some(mut row) = build::find(c, source, &version)? else {
                let row = Build {
                    id: BuildId::new(),
                    source,
                    engine: engine_of(source),
                    name: display_name(source).to_string(),
                    version,
                    channel,
                    path,
                    state: BuildState::Installing,
                    sha256: None,
                    size_bytes: 0,
                    capabilities: capabilities(source),
                    installed_at: None,
                    last_referenced_at: None,
                };
                // Captured now: once the new Build installs it is the current one.
                let previous = previous_version(c, &row)?;
                build::insert(c, &row)?;

                return Ok((row, Some(previous), false));
            };

            // A running install owns its row and writes it back when done, so leave that one alone.
            // A Camoufox release joins a channel only by passing the compatibility suite of this
            // brr version; an older record sends it back through the suite.
            let join = match source {
                BuildSource::Camoufox => camoufox_join(&row, brr_version),
                _ => Join::Yes,
            };
            let wants_channel =
                row.channel.is_none() && channel.is_some() && row.state != BuildState::Installing;
            if wants_channel && join == Join::Yes {
                row.channel = channel;
                build::update(c, &row)?;
            } else if wants_channel && join == Join::Regate && row.state == BuildState::Installed {
                // Set in memory only: the gate decides whether it sticks.
                row.channel = channel;
                let previous = previous_version(c, &row)?;

                return Ok((row, Some(previous), true));
            }
            if row.state != BuildState::Failed {
                return Ok((row, None, false));
            }
            row.state = BuildState::Installing;
            let previous = previous_version(c, &row)?;
            build::update(c, &row)?;

            Ok((row, Some(previous), false))
        })
        .await?;

    if let (Some(previous), true) = (&started, regate) {
        let (app, row, previous) = (Arc::clone(app), row.clone(), previous.clone());
        tokio::spawn(async move {
            if let Err(err) = finish_install(&app, row, previous).await {
                error!(
                    error = format!("{err:#}"),
                    "recording the compatibility re-check failed"
                );
            }
        });
    } else if let Some(previous) = started {
        // Its own task: a client that disconnects must not cancel a half-written download.
        tokio::spawn(run_install(Arc::clone(app), row.clone(), release, previous));
    }

    Ok(row)
}

#[derive(Debug, PartialEq)]
enum Join {
    Yes,
    No,
    Regate,
}

fn camoufox_join(row: &Build, brr_version: &str) -> Join {
    match row.path.parent().and_then(read_compat) {
        Some(record) if record.brr_version == brr_version => match record.passed {
            true => Join::Yes,
            false => Join::No,
        },
        _ => Join::Regate,
    }
}

fn previous_version(c: &Connection, row: &Build) -> Result<Option<String>, Error> {
    let Some(name) = &row.channel else {
        return Ok(None);
    };
    let channel = Channel {
        source: row.source,
        name: name.clone(),
    };

    Ok(current_for_channel(&build::all(c)?, &channel).map(|b| b.version))
}

async fn run_install(app: Arc<App>, mut row: Build, release: Release, previous: Option<String>) {
    let (progress, mut updates) = mpsc::channel::<Progress>(PROGRESS_BUFFER);
    let forward = async {
        while let Some(update) = updates.recv().await {
            let event = EventData::BuildInstallProgress {
                build_id: row.id,
                bytes: update.bytes,
                total: update.total,
                phase: update.phase,
            };
            if let Err(err) = app.events.publish(event).await {
                warn!(build_id = %row.id, error = format!("{err:#}"), "publishing install progress failed");
            }
        }
    };
    let builds_dir = app.paths.builds_dir();
    let (result, ()) = tokio::join!(
        install_release(&app.http, &release, &builds_dir, progress),
        forward
    );

    let recorded = match result {
        Ok(installed) => {
            row.path = installed.executable;
            // None: the files were already in place, so the row keeps its recorded hash and size.
            if let Some(size_bytes) = installed.size_bytes {
                row.sha256 = installed.sha256;
                row.size_bytes = size_bytes;
            }

            finish_install(&app, row, previous).await.map(drop)
        }
        Err(err) => fail_install(&app, row, err).await,
    };
    if let Err(err) = recorded {
        error!(
            error = format!("{err:#}"),
            "recording the finished install failed"
        );
    }
}

pub async fn finish_install(
    app: &Arc<App>,
    mut build: Build,
    previous: Option<String>,
) -> Result<Build, Error> {
    // The suite launches a browser for up to a few minutes; this runs in the install task.
    let mut incompatible = None;
    if build.source == BuildSource::Camoufox {
        let outcome = match camoufox::ensure_assets(app, build.id).await {
            Ok(assets) => compat::gate(app, &build, &assets).await,
            Err(err) => Err(err),
        };
        match outcome {
            Ok(Gate::Passed) => {}
            Ok(Gate::Failed(failure)) => {
                build.channel = None;
                incompatible = Some(failure);
            }
            Err(err) => {
                fail_install(app, build.clone(), err).await?;
                build.state = BuildState::Failed;

                return Ok(build);
            }
        }
    }

    build.installed_at = Some(now_ms());
    build.state = BuildState::Installed;
    let saved = build.clone();
    app.store.run(move |c| build::update(c, &saved)).await?;

    // The Build stays installed, pinned by version, but never joins a channel.
    if let Some(failure) = incompatible {
        app.events
            .publish(EventData::BuildInstallFailed {
                build_id: build.id,
                source: build.source,
                version: build.version.clone(),
                error: format!("compatibility suite failed at {failure}"),
            })
            .await?;

        return Ok(build);
    }

    app.events
        .publish(EventData::BuildInstalled {
            build_id: build.id,
            source: build.source,
            version: build.version.clone(),
        })
        .await?;
    if let Some(channel) = &build.channel
        && previous
            .as_deref()
            .is_none_or(|from| compare_versions(&build.version, from) == Ordering::Greater)
    {
        app.events
            .publish(EventData::BuildChannelMoved {
                source: build.source,
                channel: channel.clone(),
                from: previous,
                to: build.version.clone(),
            })
            .await?;
    }

    Ok(build)
}

async fn fail_install(app: &Arc<App>, mut build: Build, err: Error) -> Result<(), Error> {
    warn!(build_id = %build.id, error = format!("{err:#}"), "build install failed");
    build.state = BuildState::Failed;
    let event = EventData::BuildInstallFailed {
        build_id: build.id,
        source: build.source,
        version: build.version.clone(),
        error: err.to_string(),
    };

    app.store.run(move |c| build::update(c, &build)).await?;
    app.events.publish(event).await.map(drop)
}

pub fn current_for_channel(builds: &[Build], channel: &Channel) -> Option<Build> {
    builds
        .iter()
        .filter(|b| {
            b.source == channel.source
                && b.state == BuildState::Installed
                && b.channel.as_deref() == Some(&channel.name)
        })
        .max_by(|a, b| compare_versions(&a.version, &b.version))
        .cloned()
}

struct Users {
    all: Vec<Build>,
    followed: Vec<Channel>,
    templates: Vec<(TemplateId, String, BuildChoice)>,
}

impl Users {
    fn load(c: &Connection) -> Result<Users, Error> {
        Ok(Users {
            all: build::all(c)?,
            followed: profile::followed_channels(c)?,
            templates: template::user_build_choices(c)?,
        })
    }

    fn is_current(&self, channel: &Channel, build: &Build) -> bool {
        current_for_channel(&self.all, channel).is_some_and(|current| current.id == build.id)
    }
}

pub fn view(c: &Connection, build: Build) -> Result<BuildView, Error> {
    view_with(c, build, &Users::load(c)?)
}

pub fn views(c: &Connection, builds: Vec<Build>) -> Result<Vec<BuildView>, Error> {
    let users = Users::load(c)?;

    builds
        .into_iter()
        .map(|build| view_with(c, build, &users))
        .collect()
}

fn view_with(c: &Connection, build: Build, users: &Users) -> Result<BuildView, Error> {
    let referenced_by = references(c, &build, users)?;

    Ok(BuildView {
        build,
        referenced_by,
    })
}

fn references(c: &Connection, build: &Build, users: &Users) -> Result<Vec<BuildReference>, Error> {
    let mut found: Vec<BuildReference> = profile::pinning_build(c, &build.id)?
        .into_iter()
        .map(|(id, name)| BuildReference {
            kind: BuildRefKind::Profile,
            id: id.to_string(),
            label: name,
        })
        .collect();
    found.extend(
        users
            .templates
            .iter()
            .filter(|(_, _, choice)| match choice {
                BuildChoice::BuildId(id) => *id == build.id,
                BuildChoice::Channel(channel) => users.is_current(channel, build),
            })
            .map(|(id, name, _)| BuildReference {
                kind: BuildRefKind::Template,
                id: id.to_string(),
                label: name.clone(),
            }),
    );
    found.extend(
        users
            .followed
            .iter()
            .filter(|channel| users.is_current(channel, build))
            .map(|channel| BuildReference {
                kind: BuildRefKind::Channel,
                id: channel.to_string(),
                label: channel.to_string(),
            }),
    );
    found.extend(
        instance::using_build(c, &build.id)?
            .into_iter()
            .map(|id| BuildReference {
                kind: BuildRefKind::Instance,
                id: id.to_string(),
                label: id.to_string(),
            }),
    );

    Ok(found)
}

pub async fn referenced_by(
    app: &Arc<App>,
    build_id: &BuildId,
) -> Result<Vec<BuildReference>, Error> {
    let build_id = *build_id;
    let found = app
        .store
        .run(move |c| view(c, build::get(c, &build_id)?))
        .await?;

    Ok(found.referenced_by)
}

pub async fn remove(app: &Arc<App>, build_id: &BuildId) -> Result<Build, Error> {
    let build_id = *build_id;
    let builds_dir = app.paths.builds_dir();
    let (removed, leftovers) = app
        .store
        .run(move |c| {
            let found = build::get(c, &build_id)?;
            if found.source == BuildSource::System {
                return Err(Error::InvalidArgument {
                    field: "build_id".to_string(),
                    message: "system browsers are detected, not installed; uninstall them with your package manager".to_string(),
                });
            }
            if found.state == BuildState::Installing {
                return Err(Error::Conflict {
                    reason: ConflictReason::Referenced,
                    detail: Some("install in progress".to_string()),
                });
            }

            let references = references(c, &found, &Users::load(c)?)?;
            if !references.is_empty() {
                let listed: Vec<String> = references
                    .iter()
                    .map(|r| format!("{} {}", r.kind, r.label))
                    .collect();

                return Err(Error::Conflict {
                    reason: ConflictReason::Referenced,
                    detail: Some(listed.join(REFERENCE_SEPARATOR)),
                });
            }

            let leftovers = removable_paths(&builds_dir, &found)?;
            build::delete(c, &build_id)?;

            Ok((found, leftovers))
        })
        .await?;

    task::spawn_blocking(move || {
        leftovers.iter().try_for_each(|path| {
            if path.is_dir() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            }
        })
    })
    .await
    .map_err(internal)?
    .map_err(internal)?;
    app.events
        .publish(EventData::BuildRemoved {
            build_id: removed.id,
            source: removed.source,
            version: removed.version.clone(),
        })
        .await?;

    Ok(removed)
}

// Deleting what a database row points at is a data-loss path: only the <source>/<version>
// directory that really sits under the builds dir may go, plus the leftovers of its own download.
fn removable_paths(builds_dir: &Path, build: &Build) -> Result<Vec<PathBuf>, Error> {
    let Ok(root) = builds_dir.canonicalize() else {
        return Ok(Vec::new());
    };
    let source_dir = root.join(build.source.as_str());
    let version_dir = source_dir.join(&build.version);
    let mut paths = Vec::new();

    if let Ok(path) = build.path.canonicalize() {
        if !path.starts_with(&version_dir) {
            return Err(internal(format!(
                "build path {} is outside {}",
                path.display(),
                version_dir.display()
            )));
        }
        paths.push(version_dir);
    }

    if build.state == BuildState::Failed
        && let Ok(entries) = fs::read_dir(&source_dir)
    {
        let prefix = format!("{}.", build.version);
        paths.extend(entries.filter_map(Result::ok).filter_map(|entry| {
            let name = entry.file_name();
            let rest = name.to_str()?.strip_prefix(&prefix)?;
            // A longer version such as 1.2.3 must not claim the files of 1.2.
            let own = rest == TMP_SUFFIX
                || (rest.ends_with(PART_SUFFIX) && !rest.starts_with(|c: char| c.is_ascii_digit()));

            own.then(|| entry.path())
        }));
    }

    Ok(paths)
}

#[cfg(test)]
mod tests;
