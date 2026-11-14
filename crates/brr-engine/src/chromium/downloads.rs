use std::path::Path;
use std::sync::PoisonError;

use brr_cdp::{Cdp, Downstream, Outbound};
use brr_core::{Error, TargetId};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{Chromium, Inner, internal};
use crate::{DownloadState, DriverEvent};

const WILL_BEGIN: &str = "Browser.downloadWillBegin";
const PROGRESS: &str = "Browser.downloadProgress";
const MAX_FILENAME_CHARS: usize = 200;
const DEFAULT_FILENAME: &str = "download";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WillBegin {
    frame_id: String,
    guid: String,
    url: String,
    suggested_filename: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    guid: String,
    total_bytes: f64,
    received_bytes: f64,
    state: String,
}

// Files land at `dir/<guid>`; the page-chosen name only travels in events.
pub(super) async fn configure(cdp: &Cdp, dir: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(dir).map_err(internal)?;
    cdp.call(
        None,
        "Browser.setDownloadBehavior",
        json!({ "behavior": "allowAndName", "downloadPath": dir, "eventsEnabled": true }),
    )
    .await?;

    Ok(())
}

// The page chooses the filename, so nothing outside [A-Za-z0-9._ -] gets through.
pub(crate) fn sanitize(filename: &str) -> String {
    let clean: String = filename
        .chars()
        .take(MAX_FILENAME_CHARS)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ' ' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();

    // `.` and `..` would be path segments in an Artifact key.
    if clean.chars().all(|c| c == '.') {
        DEFAULT_FILENAME.to_owned()
    } else {
        clean
    }
}

fn owner(inner: &Inner, frame_id: &str) -> Option<TargetId> {
    inner
        .tabs
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|(_, tab)| tab.state.owns_frame(frame_id))
        .map(|(target_id, _)| target_id.clone())
}

pub(super) fn translate(inner: &Inner, method: &str, params: &Value) -> Option<DriverEvent> {
    match method {
        WILL_BEGIN => {
            let begin = WillBegin::deserialize(params).ok()?;

            Some(DriverEvent::DownloadStarted {
                target_id: owner(inner, &begin.frame_id),
                guid: begin.guid,
                url: begin.url,
                filename: sanitize(&begin.suggested_filename),
            })
        }
        PROGRESS => {
            let progress = Progress::deserialize(params).ok()?;
            let state = match progress.state.as_str() {
                "inProgress" => DownloadState::InProgress,
                "completed" => DownloadState::Completed,
                "canceled" => DownloadState::Canceled,
                _ => return None,
            };

            Some(DriverEvent::DownloadProgress {
                guid: progress.guid,
                received_bytes: progress.received_bytes as u64,
                total_bytes: Some(progress.total_bytes as u64).filter(|total| *total > 0),
                state,
            })
        }
        _ => None,
    }
}

pub(super) fn is_download_event(method: &str) -> bool {
    matches!(method, WILL_BEGIN | PROGRESS)
}

impl Chromium {
    // The download may belong to any context, and Browser.cancelDownload only looks in the one it
    // is given, so every context is asked.
    pub(crate) async fn cancel_download(&self, guid: &str) -> Result<(), Error> {
        let contexts = self
            .inner
            .cdp
            .call(None, "Target.getBrowserContexts", json!({}))
            .await?;
        let context_ids = std::iter::once(&contexts["defaultBrowserContextId"]).chain(
            contexts["browserContextIds"]
                .as_array()
                .into_iter()
                .flatten(),
        );

        let mut canceled = Err(internal("the browser reported no browser context"));
        for context_id in context_ids {
            let answer = self
                .inner
                .cdp
                .call(
                    None,
                    "Browser.cancelDownload",
                    json!({ "guid": guid, "browserContextId": context_id }),
                )
                .await
                .map(|_| ());
            if canceled.is_err() {
                canceled = answer;
            }
        }

        canceled
    }

    pub(crate) async fn downstream(&self) -> Result<(Downstream, mpsc::Receiver<Outbound>), Error> {
        self.inner.cdp.attach_downstream().await
    }
}
