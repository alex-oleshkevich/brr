use brr_core::TargetId;
use brr_proto::Incoming;
use serde::Deserialize;
use serde_json::Value;
use tracing::debug;

use crate::SessionId;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq)]
pub enum JugglerEvent {
    AttachedToTarget {
        session_id: SessionId,
        target_id: TargetId,
        browser_context_id: Option<String>,
        opener_id: Option<TargetId>,
    },
    DetachedFromTarget {
        session_id: SessionId,
        target_id: TargetId,
    },
    DownloadCreated {
        uuid: String,
        page_target_id: TargetId,
        url: String,
        suggested_file_name: String,
    },
    DownloadFinished {
        uuid: String,
        canceled: bool,
        error: Option<String>,
    },
    Other {
        method: String,
        params: Value,
        session_id: Option<SessionId>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfo {
    target_id: TargetId,
    browser_context_id: Option<String>,
    opener_id: Option<TargetId>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachedParams {
    session_id: SessionId,
    target_info: TargetInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetachedParams {
    session_id: SessionId,
    target_id: TargetId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadCreatedParams {
    uuid: String,
    page_target_id: TargetId,
    url: String,
    suggested_file_name: String,
}

#[derive(Deserialize)]
struct DownloadFinishedParams {
    uuid: String,
    canceled: Option<bool>,
    error: Option<String>,
}

// Unknown methods hand their params back untouched, so the common case allocates nothing.
enum Undecoded {
    Unknown(Value),
    Malformed(serde_json::Error),
}

impl From<serde_json::Error> for Undecoded {
    fn from(error: serde_json::Error) -> Undecoded {
        Undecoded::Malformed(error)
    }
}

pub fn decode(incoming: Incoming) -> JugglerEvent {
    let Incoming {
        method,
        params,
        session_id,
    } = incoming;
    let session_id = session_id.map(SessionId);

    // Root events carry no session; the same names on a page session belong to the page.
    let decoded = if session_id.is_none() {
        decode_root(&method, params)
    } else {
        Err(Undecoded::Unknown(params))
    };

    match decoded {
        Ok(event) => event,
        Err(Undecoded::Unknown(params)) => JugglerEvent::Other {
            method,
            params,
            session_id,
        },
        Err(Undecoded::Malformed(error)) => {
            debug!(%error, method, "Juggler event did not match its expected shape");
            JugglerEvent::Other {
                method,
                params: Value::Null,
                session_id,
            }
        }
    }
}

fn decode_root(method: &str, params: Value) -> Result<JugglerEvent, Undecoded> {
    Ok(match method {
        "Browser.attachedToTarget" => {
            let attached = serde_json::from_value::<AttachedParams>(params)?;
            JugglerEvent::AttachedToTarget {
                session_id: attached.session_id,
                target_id: attached.target_info.target_id,
                browser_context_id: attached.target_info.browser_context_id,
                opener_id: attached.target_info.opener_id,
            }
        }
        "Browser.detachedFromTarget" => {
            let detached = serde_json::from_value::<DetachedParams>(params)?;
            JugglerEvent::DetachedFromTarget {
                session_id: detached.session_id,
                target_id: detached.target_id,
            }
        }
        "Browser.downloadCreated" => {
            let created = serde_json::from_value::<DownloadCreatedParams>(params)?;
            JugglerEvent::DownloadCreated {
                uuid: created.uuid,
                page_target_id: created.page_target_id,
                url: created.url,
                suggested_file_name: created.suggested_file_name,
            }
        }
        "Browser.downloadFinished" => {
            let finished = serde_json::from_value::<DownloadFinishedParams>(params)?;
            JugglerEvent::DownloadFinished {
                uuid: finished.uuid,
                canceled: finished.canceled.unwrap_or(false),
                error: finished.error,
            }
        }
        _ => return Err(Undecoded::Unknown(params)),
    })
}
