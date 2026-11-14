use std::{sync::Arc, time::Duration};

use brr_builds::Want;
use brr_core::{BuildSource, Error, clock, now_ms};
use brr_store::profile;
use tokio::time;
use tracing::warn;

use super::install;
use crate::app::App;

pub fn spawn(app: Arc<App>) {
    // Held weakly: a strong reference would keep the daemon lock after the last owner drops the App.
    let app = Arc::downgrade(&app);

    tokio::spawn(async move {
        loop {
            let Some(strong) = app.upgrade() else {
                return;
            };
            let now = now_ms();
            let next = clock::next_local_time(now, strong.settings().builds.check_at, None);
            drop(strong);
            time::sleep(Duration::from_millis(
                u64::try_from(next - now).unwrap_or(0),
            ))
            .await;

            let Some(strong) = app.upgrade() else {
                return;
            };
            check_all(&strong).await;
        }
    });
}

pub async fn check_all(app: &Arc<App>) -> Vec<Result<(), Error>> {
    let followed = match app.store.run(|c| profile::followed_channels(c)).await {
        Ok(followed) => followed,
        Err(err) => {
            warn!(
                error = format!("{err:#}"),
                "reading the followed channels failed"
            );

            return vec![Err(err)];
        }
    };

    let mut results = Vec::new();
    for channel in followed
        .into_iter()
        .filter(|c| !matches!(c.source, BuildSource::System | BuildSource::Camoufox))
    {
        // An installed version is returned as it is, so only a new one downloads.
        let result = install(app, channel.source, Want::Channel(channel.name.clone()))
            .await
            .map(drop);
        if let Err(err) = &result {
            warn!(source = %channel.source, channel = channel.name, error = format!("{err:#}"), "channel check failed");
        }
        results.push(result);
    }

    results
}
