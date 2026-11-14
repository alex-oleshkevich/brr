use brr_cdp::SessionId;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::watch;

use super::capped;
use super::state::PageState;

pub const JS_DIALOG_OPENING: &str = "Page.javascriptDialogOpening";
pub const JS_DIALOG_CLOSED: &str = "Page.javascriptDialogClosed";
const MAX_DIALOG_CHARS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogType {
    Alert,
    Confirm,
    Prompt,
    Beforeunload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    pub dialog_type: DialogType,
    pub message: String,
    pub default_prompt: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Opening {
    #[serde(rename = "type")]
    kind: String,
    message: String,
    default_prompt: Option<String>,
}

impl PageState {
    pub fn dialog(&self) -> watch::Receiver<Option<Dialog>> {
        self.dialog.subscribe()
    }

    // The session the dialog opened on answers it: an out-of-process iframe has its own.
    pub(crate) fn dialog_session(&self) -> Option<SessionId> {
        self.dialog_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) fn apply_dialog(&self, session_id: &SessionId, method: &str, params: &Value) {
        if method == JS_DIALOG_CLOSED {
            self.dialog.send_replace(None);

            return;
        }
        let Ok(opening) = Opening::deserialize(params) else {
            return;
        };
        let dialog_type = match opening.kind.as_str() {
            "alert" => DialogType::Alert,
            "confirm" => DialogType::Confirm,
            "prompt" => DialogType::Prompt,
            "beforeunload" => DialogType::Beforeunload,
            _ => return,
        };

        *self
            .dialog_session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(session_id.clone());
        self.dialog.send_replace(Some(Dialog {
            dialog_type,
            message: self.redact_owned(capped(&opening.message, MAX_DIALOG_CHARS)),
            default_prompt: self.redact_owned(capped(
                &opening.default_prompt.unwrap_or_default(),
                MAX_DIALOG_CHARS,
            )),
        }));
    }
}
