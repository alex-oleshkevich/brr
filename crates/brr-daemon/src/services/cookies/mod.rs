use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use brr_core::{
    ConflictReason, Cookie, CookieFormat, Engine, Error, EventData, Instance, InstanceId,
    InstanceState, Profile, ProfileId, ProfileKind, cookie,
};
use brr_engine::{profile_files, read_cookies_offline, write_cookies_offline};
use brr_store::{Connection, instance};
use tokio::task;
use tracing::info;

use crate::{app::App, internal, ops::invalid, services::profiles::resolve_build};

const CHROMIUM_DEFAULT_DIR: &str = "Default";

pub enum Target {
    Instance(InstanceId),
    Files(PathBuf, Engine),
}

pub struct Applied {
    pub imported: u32,
    // Session cookies a stopped Camoufox Profile stores as 400-day cookies (P4).
    pub converted: u32,
}

fn is_running(state: InstanceState) -> bool {
    matches!(
        state,
        InstanceState::Running | InstanceState::Idle | InstanceState::Reaping
    )
}

pub(crate) fn profile_running() -> Error {
    Error::Conflict {
        reason: ConflictReason::ProfileRunning,
        detail: None,
    }
}

// An export reads the profile's SQLite files for as long as it runs; an offline write meanwhile
// would tear them in the archive. The export guard is taken before its own check of the store.
pub(crate) fn ensure_not_exporting(app: &App, profile_id: &ProfileId) -> Result<(), Error> {
    if app.exports.is_exporting(profile_id) {
        return Err(Error::Conflict {
            reason: ConflictReason::ProfileRunning,
            detail: Some("the profile is being exported; try again when it is done".to_owned()),
        });
    }

    Ok(())
}

fn live_instances(c: &Connection, profile: &Profile) -> Result<Vec<Instance>, Error> {
    Ok(instance::non_final(c)?
        .into_iter()
        .filter(|row| row.profile_id == Some(profile.id))
        .collect())
}

// The profile's files, when it has any, and the engine whose browser wrote them.
pub(crate) fn files_dir(c: &Connection, profile: &Profile) -> Result<(PathBuf, Engine), Error> {
    let dir = match (&profile.data_dir, profile.kind) {
        (Some(dir), ProfileKind::Persistent) => dir.clone(),
        _ => {
            return Err(invalid(
                "profile",
                "ephemeral profiles keep no data while stopped; pass a running instance",
            ));
        }
    };

    let engine = engine_of(c, profile, &dir)?;

    Ok((dir, engine))
}

fn engine_of(c: &Connection, profile: &Profile, dir: &Path) -> Result<Engine, Error> {
    match resolve_build(c, &profile.settings.build) {
        Ok(build) => Ok(build.engine),
        Err(Error::BuildMissing { .. }) if dir.join(CHROMIUM_DEFAULT_DIR).exists() => {
            Ok(Engine::Chromium)
        }
        Err(error) => Err(error),
    }
}

// The running check and the choice of offline mode read the same rows in one transaction: a
// browser undoes writes to the files of a running profile when it exits.
pub async fn select(
    app: &Arc<App>,
    profile: &Profile,
    instance_id: Option<&InstanceId>,
) -> Result<Target, Error> {
    let profile = profile.clone();
    let wanted = instance_id.copied();

    app.store
        .run(move |c| {
            let live = live_instances(c, &profile)?;

            if let Some(wanted) = wanted {
                let row = live.iter().find(|row| row.id == wanted).ok_or_else(|| {
                    invalid(
                        "instance_id",
                        "the instance does not belong to this profile or has ended",
                    )
                })?;
                if !is_running(row.state) {
                    return Err(profile_running());
                }

                return Ok(Target::Instance(wanted));
            }

            if live.iter().any(|row| !is_running(row.state)) {
                return Err(profile_running());
            }
            match live.as_slice() {
                [] => files_dir(c, &profile).map(|(dir, engine)| Target::Files(dir, engine)),
                [only] => Ok(Target::Instance(only.id)),
                many => Err(invalid(
                    "instance_id",
                    &format!(
                        "profile has {} running instances; pass instance_id",
                        many.len()
                    ),
                )),
            }
        })
        .await
}

pub async fn fetch(app: &Arc<App>, target: &Target) -> Result<Vec<Cookie>, Error> {
    match target {
        Target::Instance(id) => app.instances.cookies(id).await,
        Target::Files(dir, engine) => {
            let (dir, engine) = (dir.clone(), *engine);

            task::spawn_blocking(move || match engine {
                Engine::Camoufox => read_cookies_offline(&dir),
                Engine::Chromium => profile_files::read_cookies(&dir),
            })
            .await
            .map_err(internal)?
        }
    }
}

pub fn render(format: CookieFormat, cookies: &[Cookie]) -> String {
    match format {
        CookieFormat::Json => cookie::to_storage_state(cookies),
        CookieFormat::Netscape => cookie::to_netscape(cookies),
    }
}

pub struct Exported {
    pub data: String,
    pub count: u32,
}

pub async fn export(
    app: &Arc<App>,
    profile: &Profile,
    format: CookieFormat,
    instance_id: Option<&InstanceId>,
) -> Result<Exported, Error> {
    let target = select(app, profile, instance_id).await?;
    let all = fetch(app, &target).await?;

    Ok(Exported {
        data: render(format, &all),
        count: u32::try_from(all.len()).unwrap_or(u32::MAX),
    })
}

pub fn parse(format: CookieFormat, data: &str) -> Result<Vec<Cookie>, Error> {
    match format {
        CookieFormat::Json => cookie::from_json(data),
        CookieFormat::Netscape => cookie::from_netscape(data),
    }
}

// Select once, take the lease for an Instance target, then write to that same target.
pub async fn apply(
    app: &Arc<App>,
    profile: &Profile,
    cookies: Vec<Cookie>,
    target: &Target,
) -> Result<Applied, Error> {
    let converted = match target {
        Target::Files(_, Engine::Camoufox) => cookies.iter().filter(|c| c.expires < 0.0).count(),
        _ => 0,
    };
    let imported = match target {
        Target::Instance(id) => app.instances.set_cookies(id, cookies).await?,
        Target::Files(dir, engine) => {
            let (dir, engine, profile_id) = (dir.clone(), *engine, profile.id);
            let app_ = Arc::clone(app);

            // Checked again inside the store transaction that a launch also needs, so no
            // browser can start on these files while they are written.
            app.store
                .run(move |c| {
                    if instance::active_for_profile(c, &profile_id)?.is_some() {
                        return Err(profile_running());
                    }
                    ensure_not_exporting(&app_, &profile_id)?;

                    match engine {
                        Engine::Camoufox => write_cookies_offline(&dir, &cookies),
                        Engine::Chromium => profile_files::write_cookies(&dir, &cookies),
                    }
                })
                .await?
        }
    };
    // Counts only: cookie values are session secrets.
    info!(profile_id = %profile.id, imported, "cookies imported");
    app.events
        .publish(EventData::ProfileUpdated {
            profile_id: profile.id,
            name: profile.name.clone(),
        })
        .await?;

    Ok(Applied {
        imported,
        converted: u32::try_from(converted).unwrap_or(u32::MAX),
    })
}

pub async fn import(
    app: &Arc<App>,
    profile: &Profile,
    format: CookieFormat,
    data: &str,
    instance_id: Option<&InstanceId>,
) -> Result<Applied, Error> {
    let cookies = parse(format, data)?;
    let target = select(app, profile, instance_id).await?;

    apply(app, profile, cookies, &target).await
}
