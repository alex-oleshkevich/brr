use brr_core::TargetId;
use brr_proto::Incoming;
use serde::Deserialize;
use serde::de::Error;
use serde_json::Value;
use tracing::debug;

use crate::SessionId;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    pub target_id: TargetId,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub title: String,
    pub opener_id: Option<TargetId>,
    pub parent_frame_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CdpEvent {
    TargetCreated(TargetInfo),
    TargetInfoChanged(TargetInfo),
    TargetDestroyed {
        target_id: TargetId,
    },
    TargetCrashed {
        target_id: TargetId,
    },
    AttachedToTarget {
        session_id: SessionId,
        target_info: TargetInfo,
        waiting_for_debugger: bool,
    },
    DetachedFromTarget {
        session_id: SessionId,
    },
    FrameStartedLoading {
        session_id: SessionId,
        frame_id: String,
    },
    LifecycleEvent {
        session_id: SessionId,
        frame_id: String,
        loader_id: String,
        name: String,
    },
    LoadEventFired {
        session_id: SessionId,
    },
    Other {
        method: String,
        params: Value,
        session_id: Option<SessionId>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetInfoParams {
    target_info: TargetInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetIdParams {
    target_id: TargetId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachedParams {
    session_id: SessionId,
    target_info: TargetInfo,
    waiting_for_debugger: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DetachedParams {
    session_id: SessionId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameStartedParams {
    frame_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifecycleParams {
    frame_id: String,
    loader_id: String,
    name: String,
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

pub fn decode(incoming: Incoming) -> CdpEvent {
    let Incoming {
        method,
        params,
        session_id,
    } = incoming;
    let session_id = session_id.map(SessionId);

    match decode_known(&method, params, &session_id) {
        Ok(event) => event,
        Err(Undecoded::Unknown(params)) => CdpEvent::Other {
            method,
            params,
            session_id,
        },
        Err(Undecoded::Malformed(error)) => {
            debug!(%error, method, "CDP event did not match its expected shape");
            CdpEvent::Other {
                method,
                params: Value::Null,
                session_id,
            }
        }
    }
}

fn decode_known(
    method: &str,
    params: Value,
    session_id: &Option<SessionId>,
) -> Result<CdpEvent, Undecoded> {
    // Page events travel on a flat session; without one they cannot be routed to a tab.
    let page_session = || {
        session_id
            .clone()
            .ok_or_else(|| serde_json::Error::custom("missing sessionId"))
    };

    Ok(match method {
        "Target.targetCreated" => {
            CdpEvent::TargetCreated(serde_json::from_value::<TargetInfoParams>(params)?.target_info)
        }
        "Target.targetInfoChanged" => CdpEvent::TargetInfoChanged(
            serde_json::from_value::<TargetInfoParams>(params)?.target_info,
        ),
        "Target.targetDestroyed" => CdpEvent::TargetDestroyed {
            target_id: serde_json::from_value::<TargetIdParams>(params)?.target_id,
        },
        "Target.targetCrashed" => CdpEvent::TargetCrashed {
            target_id: serde_json::from_value::<TargetIdParams>(params)?.target_id,
        },
        "Target.attachedToTarget" => {
            let attached = serde_json::from_value::<AttachedParams>(params)?;
            CdpEvent::AttachedToTarget {
                session_id: attached.session_id,
                target_info: attached.target_info,
                waiting_for_debugger: attached.waiting_for_debugger,
            }
        }
        "Target.detachedFromTarget" => CdpEvent::DetachedFromTarget {
            session_id: serde_json::from_value::<DetachedParams>(params)?.session_id,
        },
        "Page.frameStartedLoading" => CdpEvent::FrameStartedLoading {
            session_id: page_session()?,
            frame_id: serde_json::from_value::<FrameStartedParams>(params)?.frame_id,
        },
        "Page.lifecycleEvent" => {
            let lifecycle = serde_json::from_value::<LifecycleParams>(params)?;
            CdpEvent::LifecycleEvent {
                session_id: page_session()?,
                frame_id: lifecycle.frame_id,
                loader_id: lifecycle.loader_id,
                name: lifecycle.name,
            }
        }
        "Page.loadEventFired" => CdpEvent::LoadEventFired {
            session_id: page_session()?,
        },
        _ => return Err(Undecoded::Unknown(params)),
    })
}
