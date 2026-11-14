use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use brr_cdp::SessionId;
use brr_core::{Error, WaitUntil};
use serde_json::{Value, json};

use super::{Pending, internal, lifecycle_waiter, millis, wanted_lifecycle};
use crate::common::CONNECTION_CLOSED;
use crate::page::PageSession;

const FRAME_NAVIGATED: &str = "Page.frameNavigated";
const HISTORY_POLL: Duration = Duration::from_millis(50);
const SAME_DOCUMENT_EVENT: &str = "Page.navigatedWithinDocument";

impl PageSession {
    pub(crate) async fn dispatch_mouse(&self, params: Value) -> Result<(), Error> {
        self.cdp
            .call(Some(&self.session_id), "Input.dispatchMouseEvent", params)
            .await?;

        Ok(())
    }

    pub(crate) async fn dispatch_key(&self, params: Value) -> Result<(), Error> {
        self.cdp
            .call(Some(&self.session_id), "Input.dispatchKeyEvent", params)
            .await?;

        Ok(())
    }

    pub(crate) async fn insert_text(&self, text: &str) -> Result<(), Error> {
        self.cdp
            .call(
                Some(&self.session_id),
                "Input.insertText",
                json!({ "text": text }),
            )
            .await?;

        Ok(())
    }

    // `session_id` is the session the dialog opened on.
    pub(crate) async fn handle_dialog(
        &self,
        session_id: &SessionId,
        accept: bool,
        prompt_text: Option<&str>,
    ) -> Result<(), Error> {
        let mut params = json!({ "accept": accept });
        if let Some(prompt_text) = prompt_text {
            params["promptText"] = json!(prompt_text);
        }
        self.cdp
            .call(Some(session_id), "Page.handleJavaScriptDialog", params)
            .await?;

        Ok(())
    }

    // Ok(false) when there is no entry that way.
    pub(crate) async fn go_history(
        &self,
        delta: i64,
        main_frame: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<bool, Error> {
        let history = self
            .cdp
            .call(
                Some(&self.session_id),
                "Page.getNavigationHistory",
                json!({}),
            )
            .await?;
        let wanted = history["currentIndex"]
            .as_i64()
            .map(|current| current + delta);
        let entry = wanted
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| history["entries"].get(index))
            .and_then(|entry| entry["id"].as_i64());
        let (Some(entry_id), Some(wanted)) = (entry, wanted) else {
            return Ok(false);
        };

        // A page restored from the back/forward cache fires no lifecycle event, so the
        // history index and the ready state are watched as well.
        let navigation = self.navigate_and_wait(
            "Page.navigateToHistoryEntry",
            json!({ "entryId": entry_id }),
            main_frame,
            wait_until,
            timeout,
        );
        let settled = self.history_settled(wanted, wait_until);
        tokio::select! {
            result = navigation => result?,
            () = settled => {}
        }

        Ok(true)
    }

    async fn history_settled(&self, wanted: i64, wait_until: WaitUntil) {
        loop {
            tokio::time::sleep(HISTORY_POLL).await;

            let history = self
                .cdp
                .call(
                    Some(&self.session_id),
                    "Page.getNavigationHistory",
                    json!({}),
                )
                .await;
            if history.is_ok_and(|history| history["currentIndex"].as_i64() == Some(wanted))
                && let Ok(state) = self.cdp.ready_state(&self.session_id).await
                && (state == "complete"
                    || (state == "interactive" && wait_until == WaitUntil::Domcontentloaded))
            {
                return;
            }
        }
    }

    pub(crate) async fn reload(
        &self,
        main_frame: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<(), Error> {
        self.navigate_and_wait("Page.reload", json!({}), main_frame, wait_until, timeout)
            .await
    }

    // Neither call names the new loader id. The main frame's commit does, and the lifecycle
    // waiter, registered before the call, has kept what arrived before that. A same-document
    // entry commits no new document and needs no load.
    async fn navigate_and_wait(
        &self,
        method: &str,
        params: Value,
        main_frame: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<(), Error> {
        let pending = Arc::new(Mutex::new(Pending::default()));
        let lifecycle = lifecycle_waiter(
            &self.cdp,
            &self.session_id,
            wanted_lifecycle(wait_until),
            &pending,
        );
        let (committed_frame, same_document_frame) = (main_frame.to_owned(), main_frame.to_owned());
        let committed = self.cdp.wait_for(
            Some(&self.session_id),
            FRAME_NAVIGATED,
            Box::new(move |params| params["frame"]["id"] == committed_frame),
        );
        let same_document = self.cdp.wait_for(
            Some(&self.session_id),
            SAME_DOCUMENT_EVENT,
            Box::new(move |params| params["frameId"] == same_document_frame),
        );

        let navigation = async {
            self.cdp
                .call(Some(&self.session_id), method, params)
                .await?;
            let closed = |_| internal(CONNECTION_CLOSED);

            tokio::select! {
                result = same_document => result.map(|_| ()).map_err(closed),
                result = committed => {
                    let committed = result.map_err(closed)?;
                    let loader_id = committed["frame"]["loaderId"]
                        .as_str()
                        .ok_or_else(|| internal("a committed frame has no loader id"))?
                        .to_owned();
                    {
                        let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
                        if pending.seen.contains(&loader_id) {
                            return Ok(());
                        }
                        pending.expected = Some(loader_id);
                    }

                    lifecycle.await.map(|_| ()).map_err(closed)
                }
            }
        };

        tokio::time::timeout(timeout, navigation)
            .await
            .map_err(|_| Error::Timeout {
                after_ms: millis(timeout),
            })?
    }
}
