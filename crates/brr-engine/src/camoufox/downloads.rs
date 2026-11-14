use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use brr_core::{Error, TargetId};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{Camoufox, Inner, internal};
use crate::chromium::downloads::sanitize;
use crate::{DownloadState, DriverEvent};

// A page can make unbounded distinct attachment URLs; the oldest knowledge is not worth keeping.
// While a download runs, its staged file is measured this often, so the daemon's size and disk
// limits fire before it ends; with none running there is no timer.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);
const ATTACHMENT: &str = "attachment";
const MAX_REMEMBERED_LENGTHS: usize = 64;

// Files land at `dir/<uuid>`, as with Chromium; the page-chosen name only travels in events.
pub(super) async fn configure(
    juggler: &brr_juggler::Juggler,
    context_id: Option<&str>,
    dir: &Path,
) -> Result<(), Error> {
    std::fs::create_dir_all(dir).map_err(internal)?;
    let mut params = json!({
        "downloadOptions": { "behavior": "saveToDisk", "downloadsDir": dir },
    });
    if let Some(context_id) = context_id {
        params["browserContextId"] = json!(context_id);
    }
    juggler
        .call(None, "Browser.setDownloadOptions", params)
        .await?;

    Ok(())
}

// The downloads still running, by uuid, with the size their response declared.
#[derive(Default)]
pub(super) struct Active {
    downloads: HashMap<String, Option<u64>>,
    ticking: bool,
}

impl Inner {
    // Juggler reports no byte progress, so the only size known before a download ends is the
    // Content-Length of the response that carried it.
    pub(super) fn remember_length(&self, url: String, length: u64) {
        let mut lengths = self
            .download_lengths
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if lengths.len() >= MAX_REMEMBERED_LENGTHS {
            lengths.clear();
        }
        lengths.insert(url, length);
    }

    fn take_length(&self, url: &str) -> Option<u64> {
        self.download_lengths
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(url)
    }

    // The Started event, then the declared size if a response announced one.
    pub(super) fn download_created(
        self: &Arc<Self>,
        uuid: String,
        target_id: TargetId,
        url: String,
        suggested_file_name: &str,
        out: &mpsc::Sender<DriverEvent>,
    ) -> [Option<DriverEvent>; 2] {
        let total = self.take_length(&url);
        self.track(uuid.clone(), total, out);
        let started = DriverEvent::DownloadStarted {
            target_id: Some(target_id),
            guid: uuid.clone(),
            url,
            filename: sanitize(suggested_file_name),
        };
        let declared = total.map(|total| DriverEvent::DownloadProgress {
            guid: uuid,
            received_bytes: 0,
            total_bytes: Some(total),
            state: DownloadState::InProgress,
        });

        [Some(started), declared]
    }

    fn track(self: &Arc<Self>, uuid: String, total: Option<u64>, out: &mpsc::Sender<DriverEvent>) {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        active.downloads.insert(uuid, total);
        if !active.ticking {
            active.ticking = true;
            tokio::spawn(report_progress(self.clone(), out.clone()));
        }
    }

    // The staged file's size is the only byte count Juggler lets us see.
    pub(super) fn download_finished(
        &self,
        uuid: String,
        canceled: bool,
        error: Option<&str>,
    ) -> DriverEvent {
        self.active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .downloads
            .remove(&uuid);
        let received_bytes =
            std::fs::metadata(self.download_dir.join(&uuid)).map_or(0, |m| m.len());
        let state = if canceled || error.is_some() {
            DownloadState::Canceled
        } else {
            DownloadState::Completed
        };

        DriverEvent::DownloadProgress {
            guid: uuid,
            received_bytes,
            total_bytes: Some(received_bytes).filter(|total| *total > 0),
            state,
        }
    }
}

async fn report_progress(inner: Arc<Inner>, out: mpsc::Sender<DriverEvent>) {
    loop {
        tokio::time::sleep(PROGRESS_INTERVAL).await;
        let running: Vec<(String, Option<u64>)> = {
            let mut active = inner.active.lock().unwrap_or_else(PoisonError::into_inner);
            if active.downloads.is_empty() {
                active.ticking = false;
                return;
            }

            active
                .downloads
                .iter()
                .map(|(uuid, total)| (uuid.clone(), *total))
                .collect()
        };

        for (uuid, total_bytes) in running {
            let received_bytes =
                std::fs::metadata(inner.download_dir.join(&uuid)).map_or(0, |file| file.len());
            let event = DriverEvent::DownloadProgress {
                guid: uuid,
                received_bytes,
                total_bytes,
                state: DownloadState::InProgress,
            };
            if out.send(event).await.is_err() {
                inner
                    .active
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .ticking = false;

                return;
            }
        }
    }
}

fn header<'a>(headers: &'a Value, wanted: &str) -> Option<&'a str> {
    headers
        .as_array()?
        .iter()
        .find(|header| {
            header["name"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
        })
        .and_then(|header| header["value"].as_str())
}

pub(super) fn is_attachment(headers: &Value) -> bool {
    header(headers, "content-disposition").is_some_and(|value| {
        value
            .trim_start()
            .get(..ATTACHMENT.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(ATTACHMENT))
    })
}

pub(super) fn content_length(headers: &Value) -> Option<u64> {
    header(headers, "content-length")?.trim().parse().ok()
}

impl Camoufox {
    pub(crate) async fn cancel_download(&self, uuid: &str) -> Result<(), Error> {
        self.inner
            .juggler
            .call(None, "Browser.cancelDownload", json!({ "uuid": uuid }))
            .await?;

        Ok(())
    }
}
