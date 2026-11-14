use std::sync::Arc;

use brr_core::{Engine, Error, EventData, Profile, bookmark};
use brr_engine::{export_bookmarks_offline, import_bookmarks_offline, profile_files};
use brr_store::instance;
use tracing::info;

use crate::{
    app::App,
    services::cookies::{ensure_not_exporting, files_dir, profile_running},
};

// Bookmarks are only touched while the browser is stopped: it rewrites its own file on exit. The
// work runs inside the store transaction a launch also needs, so none can start meanwhile.
async fn while_stopped<T: Send + 'static>(
    app: &Arc<App>,
    profile: &Profile,
    writes: bool,
    work: impl FnOnce(&std::path::Path, Engine) -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    let (profile, owner) = (profile.clone(), Arc::clone(app));

    app.store
        .run(move |c| {
            if instance::active_for_profile(c, &profile.id)?.is_some() {
                return Err(profile_running());
            }
            if writes {
                ensure_not_exporting(&owner, &profile.id)?;
            }
            let (dir, engine) = files_dir(c, &profile)?;

            work(&dir, engine)
        })
        .await
}

pub async fn export(app: &Arc<App>, profile: &Profile) -> Result<(String, u32), Error> {
    while_stopped(app, profile, false, |dir, engine| match engine {
        Engine::Camoufox => {
            let html = export_bookmarks_offline(dir)?;
            let count = bookmark::count(&bookmark::from_html(&html)?);

            Ok((html, count))
        }
        Engine::Chromium => {
            let roots = profile_files::read_bookmarks(dir)?;

            Ok((bookmark::to_html(&roots), bookmark::count(&roots)))
        }
    })
    .await
}

pub async fn import(app: &Arc<App>, profile: &Profile, html: &str) -> Result<u32, Error> {
    let roots = bookmark::from_html(html)?;
    let html = html.to_owned();
    let imported = while_stopped(app, profile, true, move |dir, engine| match engine {
        Engine::Camoufox => import_bookmarks_offline(dir, &html),
        Engine::Chromium => profile_files::add_bookmarks(dir, &roots),
    })
    .await?;

    info!(profile_id = %profile.id, imported, "bookmarks imported");
    app.events
        .publish(EventData::ProfileUpdated {
            profile_id: profile.id,
            name: profile.name.clone(),
        })
        .await?;

    Ok(imported)
}
