use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

use brr_core::{
    ArtifactId, ArtifactKind, Download, Error, EventData, InstanceId, Paths, ProfileId, TabId,
};
use brr_engine::{DownloadState, Driver, DriverEvent};
use tokio::sync::Notify;
use tracing::warn;

use crate::{
    app::App,
    internal,
    services::{
        artifacts::{self, ArtifactOwner},
        extensions::{contained, remove_under},
        host::BYTES_PER_MB,
        quota,
    },
};

#[cfg(test)]
mod tests;

const OCTET_STREAM: &str = "application/octet-stream";
const REASON_DISK_FLOOR: &str = "disk_floor";
const REASON_TOO_LARGE: &str = "too_large";
const REASON_CANCELED: &str = "canceled";
const MIME_TYPES: [(&str, &str); 17] = [
    ("pdf", "application/pdf"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
    ("txt", "text/plain"),
    ("csv", "text/csv"),
    ("html", "text/html"),
    ("htm", "text/html"),
    ("xml", "application/xml"),
    ("js", "text/javascript"),
    ("json", "application/json"),
    ("zip", "application/zip"),
    ("mp4", "video/mp4"),
    ("mp3", "audio/mpeg"),
];

struct Tracked {
    tab_id: TabId,
    filename: String,
    // The cancelled event went out already; the browser's own `canceled` that follows is ours.
    cancelled: bool,
    // The finished file is being stored; Instance cleanup waits for it.
    storing: bool,
}

#[derive(Default)]
pub struct Tracker {
    tracked: Mutex<HashMap<String, Tracked>>,
    stored: Notify,
}

pub struct DownloadCtx {
    pub profile_id: Option<ProfileId>,
    pub instance_id: InstanceId,
    // The tab that started a Download; the browser reports it by target.
    pub tab_id: Option<TabId>,
    pub dir: PathBuf,
    pub limit_mb: Option<u64>,
    pub driver: Driver,
}

// Never awaits: it records the guid in order, then does the slow part in its own task, so the
// Instance owner's event loop is not held up.
pub fn handle(app: &Arc<App>, tracker: &Arc<Tracker>, ctx: DownloadCtx, event: DriverEvent) {
    let app = Arc::clone(app);
    let tracker = Arc::clone(tracker);
    let over_limit = |received: u64, total: Option<u64>| {
        let limit = ctx
            .limit_mb
            .unwrap_or(app.settings().retention.download_max_mb)
            * BYTES_PER_MB;

        received > limit || total.is_some_and(|total| total > limit)
    };

    match event {
        DriverEvent::DownloadStarted { guid, filename, .. } => {
            // Staged files are read and deleted by this name, so only a plain file name is trusted.
            if !is_plain_guid(&guid) {
                warn!(instance_id = %ctx.instance_id, guid, "download with an unusable id; not tracked");

                return;
            }
            let Some(tab_id) = ctx.tab_id else {
                warn!(instance_id = %ctx.instance_id, "download without a tab; not tracked");

                return;
            };
            let below_floor = quota::check_disk_floor(&app).is_err();
            tracker.entries().insert(
                guid.clone(),
                Tracked {
                    tab_id,
                    filename: filename.clone(),
                    cancelled: below_floor,
                    storing: false,
                },
            );

            tokio::spawn(async move {
                let download = download(&ctx, tab_id, filename, 0, None, None);
                publish(&app, EventData::DownloadStarted(download.clone())).await;

                if below_floor {
                    cancel(&app, &ctx, &guid, download, REASON_DISK_FLOOR).await;
                }
            });
        }
        DriverEvent::DownloadProgress {
            guid,
            received_bytes,
            total_bytes,
            state,
        } => {
            let too_large = over_limit(received_bytes, total_bytes);
            let mut entries = tracker.entries();
            let Some(tracked) = entries.get_mut(&guid) else {
                return;
            };

            match state {
                DownloadState::InProgress => {
                    if tracked.cancelled || !too_large {
                        return;
                    }
                    tracked.cancelled = true;
                    let download = download(
                        &ctx,
                        tracked.tab_id,
                        tracked.filename.clone(),
                        received_bytes,
                        None,
                        None,
                    );
                    drop(entries);

                    tokio::spawn(async move {
                        cancel(&app, &ctx, &guid, download, REASON_TOO_LARGE).await;
                    });
                }
                // The browser may finish before a cancel lands, or before any progress crossed the limit.
                DownloadState::Completed if tracked.cancelled || too_large => {
                    let was_cancelled = tracked.cancelled;
                    let Some(tracked) = entries.remove(&guid) else {
                        return;
                    };
                    drop(entries);

                    tokio::spawn(async move {
                        if !was_cancelled {
                            let download = download(
                                &ctx,
                                tracked.tab_id,
                                tracked.filename,
                                received_bytes,
                                None,
                                Some(REASON_TOO_LARGE),
                            );
                            publish(&app, EventData::DownloadCancelled(download)).await;
                        }
                        remove_staged(&ctx.dir, &guid);
                    });
                }
                DownloadState::Completed => {
                    tracked.storing = true;
                    let (tab_id, filename) = (tracked.tab_id, tracked.filename.clone());
                    drop(entries);

                    tokio::spawn(async move {
                        complete(&app, &ctx, &guid, tab_id, filename).await;
                        tracker.entries().remove(&guid);
                        tracker.stored.notify_waiters();
                    });
                }
                DownloadState::Canceled => {
                    let Some(tracked) = entries.remove(&guid) else {
                        return;
                    };
                    drop(entries);

                    tokio::spawn(async move {
                        if !tracked.cancelled {
                            let download = download(
                                &ctx,
                                tracked.tab_id,
                                tracked.filename,
                                0,
                                None,
                                Some(REASON_CANCELED),
                            );
                            publish(&app, EventData::DownloadCancelled(download)).await;
                        }
                        remove_staged(&ctx.dir, &guid);
                    });
                }
            }
        }
        _ => {}
    }
}

// remove_dir_all on a path built from an Instance id: only a directory that really sits under
// the downloads dir may go.
pub fn remove_dir(paths: &Paths, instance_id: &InstanceId) {
    let root = paths.downloads_dir();

    remove_logged(&root, &root.join(instance_id.to_string()));
}

impl Tracker {
    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Tracked>> {
        self.tracked.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // Instance cleanup must not delete a staged file that is still being stored.
    pub async fn quiesce(&self) {
        loop {
            let stored = self.stored.notified();
            if !self.entries().values().any(|tracked| tracked.storing) {
                return;
            }

            stored.await;
        }
    }
}

async fn complete(app: &Arc<App>, ctx: &DownloadCtx, guid: &str, tab_id: TabId, filename: String) {
    let owner = ArtifactOwner {
        profile_id: ctx.profile_id,
        instance_id: Some(ctx.instance_id),
        tab_id: Some(tab_id),
    };
    let mime = mime_of(&filename);

    let stored = match staged_file(&ctx.dir, guid).await {
        Ok(staged) => {
            artifacts::store_file(app, ArtifactKind::Download, owner, &filename, mime, &staged)
                .await
        }
        Err(error) => Err(error),
    };
    let event = match stored {
        Ok(artifact) => EventData::DownloadCompleted(download(
            ctx,
            tab_id,
            filename,
            artifact.size_bytes,
            Some(artifact.id),
            None,
        )),
        Err(error) => EventData::DownloadCancelled(download(
            ctx,
            tab_id,
            filename,
            0,
            None,
            Some(&format!("storage: {error}")),
        )),
    };
    publish(app, event).await;
    remove_staged(&ctx.dir, guid);
}

// What gets stored and published must be the browser's own file under the Instance's download dir.
async fn staged_file(dir: &Path, guid: &str) -> Result<PathBuf, Error> {
    let (dir, path) = (dir.to_owned(), dir.join(guid));

    tokio::task::spawn_blocking(move || contained(&dir, &path))
        .await
        .map_err(internal)??
        .ok_or_else(|| internal("the downloaded file is missing"))
}

async fn cancel(
    app: &Arc<App>,
    ctx: &DownloadCtx,
    guid: &str,
    mut download: Download,
    reason: &str,
) {
    if let Err(error) = ctx.driver.cancel_download(guid).await {
        warn!(instance_id = %ctx.instance_id, error = format!("{error:#}"), "download not cancelled in the browser");
    }

    download.reason = Some(reason.to_owned());
    publish(app, EventData::DownloadCancelled(download)).await;
}

fn download(
    ctx: &DownloadCtx,
    tab_id: TabId,
    filename: String,
    size: u64,
    artifact_id: Option<ArtifactId>,
    reason: Option<&str>,
) -> Download {
    Download {
        tab_id,
        instance_id: ctx.instance_id,
        artifact_id,
        filename,
        size,
        reason: reason.map(str::to_owned),
    }
}

async fn publish(app: &Arc<App>, data: EventData) {
    if let Err(error) = app.events.publish(data).await {
        warn!(error = format!("{error:#}"), "download event not published");
    }
}

fn mime_of(filename: &str) -> &'static str {
    let Some((_, extension)) = filename.rsplit_once('.') else {
        return OCTET_STREAM;
    };

    MIME_TYPES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(extension))
        .map_or(OCTET_STREAM, |(_, mime)| mime)
}

// The guid comes from the browser: Chromium sends a UUID, Firefox one in braces. Anything that
// is not a single plain path component (separators, `..`) is refused before it is ever joined.
fn is_plain_guid(guid: &str) -> bool {
    !guid.is_empty()
        && guid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '{' | '}'))
}

fn remove_staged(dir: &Path, guid: &str) {
    remove_logged(dir, &dir.join(guid));
}

pub(crate) fn remove_logged(root: &Path, path: &Path) {
    if let Err(error) = remove_under(root, path) {
        warn!(
            error = format!("{error:#}"),
            path = ?path,
            "download data not deleted"
        );
    }
}
