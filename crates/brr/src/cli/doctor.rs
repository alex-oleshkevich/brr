use anyhow::bail;
use brr_core::Build;
use brr_daemon::{
    Boot,
    remote::Remote,
    services::doctor::{Check, CheckStatus, DoctorEnv, checks},
};
use brr_store::{Store, build};
use serde_json::json;

use super::format::{self, Group};

pub const NAME: &str = "doctor";

const SYSTEM_DOCTOR: &str = "system.doctor";
const GROUP: &str = "checks";

// Never starts the daemon and never creates the vault or the database.
pub async fn run(boot: &Boot, json: bool) -> anyhow::Result<()> {
    let remote = Remote::new(&boot.paths.socket(&boot.settings))?;
    let checks = if remote.is_up().await {
        let output = remote.call(SYSTEM_DOCTOR, &json!({}), None).await?;

        serde_json::from_value(output["checks"].clone())?
    } else {
        local_checks(boot).await?
    };

    if json {
        format::data(&json!({ "checks": checks }).to_string())?;
    } else {
        let group = checks.iter().fold(Group::new(GROUP), |group, check| {
            group.entry(
                &check.name,
                &[status_name(check.status), check.detail.clone()],
            )
        });
        format::list(&[group])?;
    }

    if checks.iter().any(|check| check.status == CheckStatus::Fail) {
        bail!("some checks failed; each detail names its fix");
    }

    Ok(())
}

async fn local_checks(boot: &Boot) -> anyhow::Result<Vec<Check>> {
    let db_file = boot.paths.db_file();
    let builds: Vec<Build> = if db_file.exists() {
        Store::open(&db_file)?.run(|c| build::all(c)).await?
    } else {
        Vec::new()
    };
    let env = DoctorEnv {
        paths: &boot.paths,
        settings: &boot.settings,
        display_env: &boot.display_env,
        path_env: &boot.path_env,
        daemon_running: false,
    };

    Ok(checks(&env, &builds).await)
}

fn status_name(status: CheckStatus) -> String {
    match status {
        CheckStatus::Ok => "ok",
        CheckStatus::Warn => "warn",
        CheckStatus::Fail => "fail",
    }
    .to_string()
}
