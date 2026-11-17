use std::{fs, path::PathBuf, sync::Arc, time::Duration};

use brr_builds::{CompatRecord, read_compat, write_compat};
use brr_core::{Build, Display, Engine, Error, InstanceId, ProfileSettings, now_ms};
use brr_engine::{CamoufoxSpec, LaunchSpec, compat::run_compat};
use tracing::{info, warn};

use crate::{app::App, internal, services::camoufox::Assets};

const BUFFER_LINES: usize = 100;

pub enum Gate {
    Passed,
    // "<step>: <message>"; the message is missing when the record came from an earlier run.
    Failed(String),
}

// R3: a Camoufox release becomes latest-compatible only after it passes the compatibility suite.
// The result is kept in the Build directory, so a Build is checked once per brr version.
pub async fn gate(app: &Arc<App>, build: &Build, assets: &Assets) -> Result<Gate, Error> {
    let build_dir = build
        .path
        .parent()
        .ok_or_else(|| internal("the Camoufox executable has no directory"))?
        .to_owned();
    if let Some(record) = read_compat(&build_dir)
        && record.brr_version == app.version
    {
        return Ok(match record.passed {
            true => Gate::Passed,
            false => Gate::Failed(record.failed_step.unwrap_or_default()),
        });
    }

    // run_compat marks its processes with an id of its own; this one only names the work dir.
    let instance_id = InstanceId::new();
    let work_dir: PathBuf = app
        .paths
        .ephemeral_dir()
        .join(format!("compat-{instance_id}"));
    let settings = app.settings();
    let spec = LaunchSpec {
        instance_id,
        engine: Engine::Camoufox,
        executable: build.path.clone(),
        user_data_dir: work_dir.join("profile"),
        log_file: work_dir.join("browser.log"),
        display: Display::Headless,
        display_env: app.display_env.clone(),
        settings: ProfileSettings::default(),
        start_timeout: Duration::from_secs(settings.instance.start_timeout_s),
        extensions: Vec::new(),
        proxy_server: None,
        download_dir: work_dir.join("downloads"),
        console_buffer: BUFFER_LINES,
        network_buffer: BUFFER_LINES,
        camoufox: Some(CamoufoxSpec {
            fonts_dir: assets.fonts_dir.clone(),
            model: Arc::clone(&assets.model),
            geo: None,
            seed: None,
            addons: Vec::new(),
        }),
        sandbox: None,
    };

    let report = run_compat(spec).await;
    if let Err(error) = fs::remove_dir_all(&work_dir) {
        warn!(%error, dir = %work_dir.display(), "compatibility work dir not removed");
    }

    let mut failure: Option<(String, String)> = None;
    for (step, result) in &report.steps {
        match result {
            Ok(()) => info!(version = %build.version, step, "compatibility step passed"),
            Err(message) => {
                warn!(version = %build.version, step, message, "compatibility step failed");
                failure.get_or_insert_with(|| (step.clone(), message.clone()));
            }
        }
    }

    write_compat(
        &build_dir,
        &CompatRecord {
            passed: report.passed,
            checked_at: now_ms(),
            failed_step: failure.as_ref().map(|(step, _)| step.clone()),
            brr_version: app.version.to_owned(),
        },
    )?;

    Ok(match (report.passed, failure) {
        (true, _) => Gate::Passed,
        (false, Some((step, message))) => Gate::Failed(format!("{step}: {message}")),
        (false, None) => Gate::Failed("the suite ran fewer steps than it has".to_owned()),
    })
}
