use std::sync::Arc;

use brr_cdp::{Cdp, SessionId};
use brr_core::Error;
use brr_core::capture::ResourceType;
use serde_json::{Value, json};
use tracing::warn;

use crate::page::{PageState, content, glob_match};

#[cfg(test)]
mod tests;

const BLOCKED_REASON: &str = "BlockedByClient";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestRules {
    pub patterns: Vec<String>,
    pub types: Vec<ResourceType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PausedRequest {
    pub url: String,
    pub resource_type: String,
    pub frame_id: String,
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Continue,
    Fail,
}

impl RequestRules {
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty() && self.types.is_empty()
    }
}

// CDP names the kinds of Network.ResourceType; XHR and Fetch are both brr's `xhr`.
fn resource_type(name: &str) -> Option<ResourceType> {
    match name {
        "Image" => Some(ResourceType::Image),
        "Font" => Some(ResourceType::Font),
        "Media" => Some(ResourceType::Media),
        "Stylesheet" => Some(ResourceType::Stylesheet),
        "Script" => Some(ResourceType::Script),
        "XHR" | "Fetch" => Some(ResourceType::Xhr),
        _ => None,
    }
}

// The Network.ResourceType names of one of brr's kinds.
pub(crate) fn cdp_resource_types(kind: ResourceType) -> &'static [&'static str] {
    match kind {
        ResourceType::Image => &["Image"],
        ResourceType::Font => &["Font"],
        ResourceType::Media => &["Media"],
        ResourceType::Stylesheet => &["Stylesheet"],
        ResourceType::Script => &["Script"],
        ResourceType::Xhr => &["XHR", "Fetch"],
    }
}

pub fn verdict(rules: &RequestRules, request: &PausedRequest) -> Verdict {
    let blocked_type =
        resource_type(&request.resource_type).is_some_and(|kind| rules.types.contains(&kind));
    let blocked_url = rules
        .patterns
        .iter()
        .any(|pattern| glob_match(pattern, &request.url));

    if blocked_type || blocked_url {
        Verdict::Fail
    } else {
        Verdict::Continue
    }
}

pub(crate) async fn set_enabled(
    cdp: &Cdp,
    session_id: &SessionId,
    state: &PageState,
) -> Result<(), Error> {
    if state.intercepting() {
        let patterns = json!({ "patterns": state.fetch_patterns() });
        cdp.call(Some(session_id), "Fetch.enable", patterns).await?;
    } else {
        cdp.call(Some(session_id), "Fetch.disable", json!({}))
            .await?;
    }

    Ok(())
}

// For a session that attaches while rules are active.
pub(crate) async fn enable_if_intercepting(
    cdp: &Cdp,
    session_id: &SessionId,
    state: &PageState,
) -> Result<(), Error> {
    if state.intercepting() {
        set_enabled(cdp, session_id, state).await?;
    }

    Ok(())
}

// Every paused request must be answered or its page hangs, so any failure continues it. Runs
// in its own task: the event loop must never wait on a call.
pub(super) fn paused(cdp: Cdp, session_id: SessionId, state: Arc<PageState>, params: Value) {
    tokio::spawn(async move {
        let Some(request_id) = params["requestId"].as_str() else {
            return;
        };
        let frame_id = params["frameId"].as_str().unwrap_or_default().to_owned();
        let request = PausedRequest {
            url: params["request"]["url"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            resource_type: params["resourceType"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            source_url: state
                .frame(&frame_id)
                .map(|frame| frame.url)
                .filter(|url| !url.is_empty()),
            frame_id,
        };

        if content::verdict(&state, &request).await == Verdict::Fail {
            let params = json!({ "requestId": request_id, "errorReason": BLOCKED_REASON });
            match cdp
                .call(Some(&session_id), "Fetch.failRequest", params)
                .await
            {
                Ok(_) => return,
                Err(error) => warn!(%error, "blocking a request failed"),
            }
        }

        let params = json!({ "requestId": request_id });
        if let Err(error) = cdp
            .call(Some(&session_id), "Fetch.continueRequest", params)
            .await
        {
            warn!(%error, "paused request not continued");
        }
    });
}
