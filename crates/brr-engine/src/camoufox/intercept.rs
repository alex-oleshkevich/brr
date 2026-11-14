use std::sync::Arc;

use brr_core::Error;
use serde_json::{Value, json};
use tracing::warn;

use super::network::resource_type;
use super::{Camoufox, PageSession};
use crate::chromium::fetch::{PausedRequest, Verdict};
use crate::page::ContentOptions;
use crate::page::{PageState, content};

const BLOCKED_CODE: &str = "blockedbyclient";

impl PageSession {
    // Juggler intercepts every request of the Tab, frames and workers included.
    pub(crate) async fn set_intercepting(&self, enabled: bool) -> Result<(), Error> {
        self.call(
            "Network.setRequestInterception",
            json!({ "enabled": enabled }),
        )
        .await?;

        Ok(())
    }
}

// Every intercepted request must be answered or its page hangs, so any failure resumes it. Runs
// in its own task: the event loop must never wait on a call.
pub(super) fn paused(session: PageSession, state: Arc<PageState>, params: &Value) {
    let Some(request_id) = params["requestId"].as_str().map(str::to_owned) else {
        return;
    };
    let frame_id = params["frameId"].as_str().unwrap_or_default().to_owned();
    let request = PausedRequest {
        url: params["url"].as_str().unwrap_or_default().to_owned(),
        resource_type: resource_type(params["cause"].as_str().unwrap_or_default()).to_owned(),
        source_url: state
            .frame(&frame_id)
            .map(|frame| frame.url)
            .filter(|url| !url.is_empty()),
        frame_id,
    };

    tokio::spawn(async move {
        if content::verdict(&state, &request).await == Verdict::Fail {
            let params = json!({ "requestId": request_id, "errorCode": BLOCKED_CODE });
            match session
                .call("Network.abortInterceptedRequest", params)
                .await
            {
                Ok(_) => return,
                Err(error) => warn!(%error, "blocking a request failed"),
            }
        }

        let params = json!({ "requestId": request_id });
        if let Err(error) = session
            .call("Network.resumeInterceptedRequest", params)
            .await
        {
            warn!(%error, "intercepted request not resumed");
        }
    });
}

impl Camoufox {
    // Interception on the browser context covers a Tab from its first request and the popups it
    // opens, which the per-Tab switch reaches only after the Tab has attached. Juggler never
    // intercepts a redirect hop (a platform limit), so a URL rule can be bypassed through one.
    pub(crate) async fn set_context_interception(
        &self,
        context_id: Option<&str>,
        options: &ContentOptions,
    ) -> Result<(), Error> {
        let mut params = json!({ "enabled": options.adblock.is_some() });
        if let Some(context_id) = context_id {
            params["browserContextId"] = json!(context_id);
        }
        self.inner
            .juggler
            .call(None, "Browser.setRequestInterception", params)
            .await?;

        Ok(())
    }
}
