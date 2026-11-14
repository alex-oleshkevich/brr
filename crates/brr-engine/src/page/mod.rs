mod actions;
mod capture;
pub(crate) mod content;
mod devices;
mod dialogs;
mod emulation;
mod frames;
mod input;
mod login;
mod network;
mod read;
mod screencast;
mod session;
mod snapshot;
mod state;

use std::sync::Arc;
use std::time::{Duration, Instant};

use brr_cdp::{Cdp, SessionId};
use brr_core::{Error, TargetId};
use serde_json::{Value, json};

pub use actions::MAX_UPLOAD_BYTES;
pub(crate) use actions::glob_match;
pub use capture::{Clip, MAX_CAPTURE_BYTES, ScreenshotOptions};
pub use content::{ADBLOCK_VERDICT_TIMEOUT, ContentOptions};
pub use devices::{Device, device};
pub use dialogs::{Dialog, DialogType};
pub(crate) use dialogs::{JS_DIALOG_CLOSED, JS_DIALOG_OPENING};
pub use emulation::{Cleanup, EmulateOptions};
pub use input::{
    ClickOptions, KeyInput, KeyKind, Modifier, MouseButton, MouseInput, MouseKind, UploadFile,
    WaitCondition,
};
pub use login::{EvaluateLock, LoginFields};
pub use network::{ConsoleMessage, HAR_MAX_ENTRIES, NetworkEntry, StorageArea};
pub use read::{Link, Metadata};
pub(crate) use screencast::number;
pub use screencast::{FrameMetadata, Screencast, ScreencastFrame, jpeg_size};

pub(crate) use network::{cookie_params, parse_cookies};

use actions::retry_wait;
pub use state::PageState;

use crate::{Driver, Live, camoufox};

#[cfg(test)]
mod tests;

pub const UTILITY_WORLD: &str = "__brr_utility__";
pub const RETRY_BACKOFF_MS: [u64; 5] = [0, 20, 100, 100, 500];

const INJECTED_SOURCE: &str = include_str!("../../assets/injected.js");
// The object is left open: the frame sequence number is appended.
const INJECTED_OPTIONS: &str = r#"{"isUnderTest":false,"sdkLanguage":"javascript","testIdAttributeName":"data-testid","stableRafCount":1,"browserName":"BROWSER","shouldPrependErrorPrefix":false,"isUtilityWorld":true,"customEngines":[]"#;
const POLL_INTERVAL_MS: u64 = 500;
const MAX_TEXT_BYTES: usize = 10 * 1024 * 1024;
const MAX_ERROR_CHARS: usize = 4096;
const DEFAULT_EVALUATE_TIMEOUT: Duration = Duration::from_secs(30);
// Remote objects of resolved elements; releasing the group frees them.
pub const ELEMENT_GROUP: &str = "brr-elements";
const REF_FIELD: &str = "ref";
const SELECTOR_FIELD: &str = "selector";

// Returns the element, a string for a selector parse error, or the match count when several match.
const SELECT_FUNCTION: &str = r"function(selector) {
    let parsed;
    try {
        parsed = this.parseSelector(selector);
    } catch (error) {
        return String(error.message);
    }
    const elements = this.querySelectorAll(parsed, document);
    return elements.length > 1 ? elements.length : elements[0];
}";
const REF_FUNCTION: &str = r"function(ref) {
    return this.querySelector(this.parseSelector('aria-ref=' + ref), document, true);
}";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Ref(String),
    Selector(String),
}

impl Target {
    pub fn from_input(r#ref: Option<String>, selector: Option<String>) -> Result<Target, Error> {
        match (r#ref, selector) {
            (Some(r#ref), None) => Ok(Target::Ref(r#ref)),
            (None, Some(selector)) => Ok(Target::Selector(selector)),
            _ => Err(Error::InvalidArgument {
                field: REF_FIELD.to_owned(),
                message: "pass exactly one of ref or selector".to_owned(),
            }),
        }
    }
}

#[derive(Clone)]
pub(crate) struct PageSession {
    pub cdp: Cdp,
    pub session_id: SessionId,
    pub target_id: TargetId,
}

#[derive(Clone)]
pub(crate) enum Session {
    Chromium(PageSession),
    Camoufox(camoufox::PageSession),
}

#[derive(Clone)]
pub struct Page {
    session: Session,
    state: Arc<PageState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub frame_id: String,
    pub session_id: SessionId,
    pub object_id: String,
}

struct Injected {
    session_id: SessionId,
    object_id: String,
}

impl Driver {
    pub fn page(&self, target_id: &TargetId) -> Result<Page, Error> {
        match self {
            Driver::Chromium(chromium) | Driver::Live(Live { chromium, .. }) => {
                chromium.page(target_id)
            }
            Driver::Camoufox(camoufox) => camoufox.page(target_id),
        }
    }
}

// Re-run for every new document: the old world dies with it. The same name in the same frame
// answers the existing world, so this is cheap when nothing changed.
pub(crate) async fn create_world(
    cdp: &Cdp,
    session_id: &SessionId,
    frame_id: &str,
    state: &PageState,
) -> Result<i64, Error> {
    let params = json!({
        "frameId": frame_id,
        "worldName": UTILITY_WORLD,
        "grantUniveralAccess": true,
    });
    let result = cdp
        .call(Some(session_id), "Page.createIsolatedWorld", params)
        .await?;
    let context_id = result["executionContextId"]
        .as_i64()
        .ok_or_else(|| Error::Internal {
            message: "Page.createIsolatedWorld: no executionContextId".to_owned(),
        })?;
    state.set_utility(frame_id, context_id);

    Ok(context_id)
}

// A RemoteObject as JSON: its value, else its type and description.
fn value_of(remote: &mut Value) -> Value {
    if let Some(value) = remote.get_mut("value") {
        return value.take();
    }
    if remote["type"] == "undefined" {
        return Value::Null;
    }

    json!({ "type": remote["type"], "description": remote["description"] })
}

fn capped(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn cap(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }

    text
}

// `e5` in the main frame, `f3e5` in the frame with sequence number 3.
fn parse_ref(text: &str) -> Option<Option<u32>> {
    let (frame_seq, rest) = match text.strip_prefix('f') {
        Some(rest) => {
            let digits = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());

            (Some(rest[..digits].parse().ok()?), &rest[digits..])
        }
        None => (None, text),
    };
    let number = rest.strip_prefix('e')?;

    (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())).then_some(frame_seq)
}

impl Page {
    pub(crate) fn new(session: Session, state: Arc<PageState>) -> Page {
        Page { session, state }
    }

    pub fn state(&self) -> &Arc<PageState> {
        &self.state
    }

    // A call on the page's own session.
    async fn page_call(&self, method: &str, params: Value) -> Result<Value, Error> {
        self.session
            .cdp()?
            .call(Some(self.session.main_session()), method, params)
            .await
    }

    // Built once per frame document: the script is 160 KB.
    async fn injected(&self, frame_id: &str) -> Result<Injected, Error> {
        let _guard = self.state.inject.lock().await;
        let frame = self.state.frame(frame_id).ok_or_else(|| Error::NotFound {
            what: "frame".to_owned(),
        })?;
        if let Some(object_id) = frame.injected {
            return Ok(Injected {
                session_id: frame.session_id,
                object_id,
            });
        }

        let context_id = self
            .session
            .utility_world(&frame.session_id, frame_id, &self.state)
            .await?;
        // Refs of the main frame carry no prefix.
        let frame_seq = if self.state.is_main(frame_id) {
            0
        } else {
            frame.frame_seq
        };
        let options = INJECTED_OPTIONS.replace("BROWSER", self.session.browser_name());
        let expression = format!(
            "{INJECTED_SOURCE}\n;new globalThis.__brrInjectedScript(globalThis, {options},\"frameSeq\":{frame_seq}}})"
        );
        let remote = self
            .session
            .evaluate(
                &frame.session_id,
                Some(context_id),
                &expression,
                false,
                None,
            )
            .await?;
        let object_id = remote["objectId"]
            .as_str()
            .ok_or_else(|| Error::Internal {
                message: "the injected script returned no object".to_owned(),
            })?
            .to_owned();
        self.state.set_injected(frame_id, context_id, &object_id);

        Ok(Injected {
            session_id: frame.session_id,
            object_id,
        })
    }

    // The document may have changed under a cached instance before its events arrived; one
    // rebuild covers that.
    // `arguments` are CDP call arguments: by value or by object id.
    async fn call_injected_args(
        &self,
        frame_id: &str,
        declaration: &str,
        arguments: Vec<Value>,
        by_value: bool,
    ) -> Result<(SessionId, Value), Error> {
        let mut retried = false;

        loop {
            let injected = self.injected(frame_id).await?;
            let context_id = self
                .state
                .frame(frame_id)
                .and_then(|frame| frame.utility_context);
            match self
                .session
                .call_function(
                    &injected.session_id,
                    context_id,
                    &injected.object_id,
                    declaration,
                    &arguments,
                    by_value,
                )
                .await
            {
                Err(Error::Internal { .. }) if !retried => {
                    retried = true;
                    self.state.clear_injected(frame_id);
                }
                result => return result.map(|remote| (injected.session_id, remote)),
            }
        }
    }

    // The document may have changed under a cached instance before its events arrived; one
    // rebuild covers that.
    async fn call_injected(
        &self,
        frame_id: &str,
        declaration: &str,
        arguments: &[Value],
        by_value: bool,
    ) -> Result<(SessionId, Value), Error> {
        let arguments = arguments
            .iter()
            .map(|value| json!({ "value": value }))
            .collect();

        self.call_injected_args(frame_id, declaration, arguments, by_value)
            .await
    }

    pub async fn resolve(&self, target: &Target, timeout: Duration) -> Result<Element, Error> {
        match target {
            Target::Ref(text) => self.resolve_ref(text).await,
            Target::Selector(selector) => self.resolve_selector(selector, timeout).await,
        }
    }

    async fn resolve_ref(&self, text: &str) -> Result<Element, Error> {
        let frame_seq = parse_ref(text).ok_or_else(|| Error::InvalidArgument {
            field: REF_FIELD.to_owned(),
            message: "a ref looks like e5 or f2e5; take it from the latest snapshot".to_owned(),
        })?;
        let frame_id = match frame_seq {
            None => self.state.main_frame(),
            Some(seq) => self.state.frame_id_by_seq(seq).ok_or(Error::StaleRef)?,
        };

        // A throw means the document changed; either way the ref is gone.
        let (session_id, remote) = self
            .call_injected(&frame_id, REF_FUNCTION, &[json!(text)], false)
            .await
            .map_err(|error| match error {
                Error::EvaluationFailed { .. } | Error::Internal { .. } => Error::StaleRef,
                other => other,
            })?;
        let object_id = remote["objectId"].as_str().ok_or(Error::StaleRef)?;

        Ok(Element {
            frame_id,
            session_id,
            object_id: object_id.to_owned(),
        })
    }

    // Main frame only; elements inside iframes are reached by ref.
    async fn resolve_selector(&self, selector: &str, timeout: Duration) -> Result<Element, Error> {
        let deadline = Instant::now() + timeout;
        let mut attempt = 0;

        loop {
            let frame_id = self.state.main_frame();
            let found = self
                .call_injected(&frame_id, SELECT_FUNCTION, &[json!(selector)], false)
                .await;
            match &found {
                Ok((session_id, remote)) => {
                    if let Some(object_id) = remote["objectId"].as_str() {
                        return Ok(Element {
                            frame_id,
                            session_id: session_id.clone(),
                            object_id: object_id.to_owned(),
                        });
                    }
                    // Not an element: a parse error message or a match count.
                    let message = match remote["type"].as_str() {
                        Some("string") => {
                            Some(remote["value"].as_str().unwrap_or_default().to_owned())
                        }
                        Some("number") => Some(format!(
                            "{} elements match; refine the selector",
                            remote["value"]
                        )),
                        _ => None,
                    };
                    if let Some(message) = message {
                        return Err(Error::InvalidArgument {
                            field: SELECTOR_FIELD.to_owned(),
                            message,
                        });
                    }
                }
                Err(Error::EvaluationFailed { message }) => {
                    return Err(Error::EvaluationFailed {
                        message: message.clone(),
                    });
                }
                Err(_) => {}
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return match found {
                    Err(error) => Err(error),
                    Ok(_) => Err(Error::ElementNotFound),
                };
            }
            tokio::time::sleep(retry_wait(attempt).min(remaining)).await;
            attempt += 1;
        }
    }
}
