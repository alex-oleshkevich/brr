use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use brr_cdp::SessionId;
use brr_core::Error;
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};
use tokio::time::Instant;

use super::content::ContentOptions;
use super::dialogs::{Dialog, JS_DIALOG_CLOSED, JS_DIALOG_OPENING};
use super::emulation::Media;
use super::frames::{Frame, FrameTree};
use super::network::Network;
use super::screencast::FrameSender;
use crate::chromium::fetch::{PausedRequest, RequestRules, Verdict, cdp_resource_types, verdict};
use crate::common::millis;
use brr_content::Adblock;

#[cfg(test)]
mod tests;

pub(crate) const NETWORK_QUIET: Duration = Duration::from_millis(500);
const NETWORK_POLL: Duration = Duration::from_millis(50);
const FRAME_EVENTS: &str = "Page.frame";
const NAVIGATION_COMMITTED: &str = "Page.navigationCommitted";
const CONTEXT_EVENTS: &str = "Runtime.executionContext";
const REDACTED: &str = "«redacted»";

// Never Debug or Clone: a secret only ever lives here and in the page it was typed into.
struct Secret(String);

// Everything the engine knows about one Tab, shared by its event task and the tool calls.
pub struct PageState {
    frames: Mutex<FrameTree>,
    network: Mutex<Network>,
    rules: Mutex<RequestRules>,
    // Every session of the Tab (page, iframes, workers): Fetch is enabled on each of them.
    sessions: Mutex<Vec<SessionId>>,
    // Longest first, so a secret that contains another is replaced whole.
    secrets: Mutex<Vec<Secret>>,
    // A secret brr typed is in the current document. Set with the secret, cleared when the main
    // frame navigates away.
    holds_secret: AtomicBool,
    // Operations that keep `evaluate` refused until they end, across navigations.
    evaluate_locks: AtomicU32,
    // Counts main-frame navigations: a waiter reads it before an action and waits for it to move.
    navigations: watch::Sender<u64>,
    // None: the Driver's defaults, copied here in `defaults`, apply.
    content: Mutex<Option<ContentOptions>>,
    defaults: Mutex<Option<ContentOptions>>,
    // The autoconsent script registered on each Chromium session, to remove it again.
    consent_scripts: Mutex<Vec<(SessionId, String)>>,
    pub(super) media: Mutex<Media>,
    pub(super) dialog: watch::Sender<Option<Dialog>>,
    pub(super) dialog_session: Mutex<Option<SessionId>>,
    // Serializes injected-script creation so two calls never build two instances in one frame.
    pub(crate) inject: tokio::sync::Mutex<()>,
    // Woken whenever frames, contexts or the network state change, for callers that wait on them.
    pub(crate) changed: Notify,
    // Where the Tab's screencast frames go while one runs.
    pub(crate) screencast: Mutex<Option<FrameSender>>,
    // Held while a screencast starts or stops, so a restart never lands before an earlier stop.
    pub(crate) screencast_operation: Arc<tokio::sync::Mutex<()>>,
}

impl PageState {
    pub(crate) fn new(console_buffer: usize, network_buffer: usize) -> PageState {
        PageState {
            frames: Mutex::default(),
            media: Mutex::default(),
            secrets: Mutex::default(),
            holds_secret: AtomicBool::new(false),
            evaluate_locks: AtomicU32::new(0),
            navigations: watch::channel(0).0,
            content: Mutex::default(),
            defaults: Mutex::default(),
            consent_scripts: Mutex::default(),
            network: Mutex::new(Network::new(console_buffer, network_buffer)),
            rules: Mutex::default(),
            sessions: Mutex::default(),
            dialog: watch::channel(None).0,
            dialog_session: Mutex::new(None),
            inject: tokio::sync::Mutex::new(()),
            changed: Notify::new(),
            screencast: Mutex::default(),
            screencast_operation: Arc::default(),
        }
    }

    // Whether `apply` reads this method, so the event loop skips the rest cheaply.
    pub(crate) fn tracks(method: &str) -> bool {
        method.starts_with(FRAME_EVENTS)
            || method.starts_with(CONTEXT_EVENTS)
            || method == NAVIGATION_COMMITTED
            || matches!(method, JS_DIALOG_OPENING | JS_DIALOG_CLOSED)
            || Network::tracks(method)
    }

    pub(super) fn network(&self) -> MutexGuard<'_, Network> {
        self.network.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn request(&self, request_id: &str) -> Option<(String, String)> {
        self.network().request(request_id)
    }

    pub(crate) fn set_rules(&self, rules: RequestRules) {
        *self.rules.lock().unwrap_or_else(PoisonError::into_inner) = rules;
    }

    // Fetch patterns for what must be paused: all requests when a URL rule or the adblocker needs
    // to see them, otherwise only the blocked resource types.
    pub(crate) fn fetch_patterns(&self) -> Value {
        let rules = self.rules.lock().unwrap_or_else(PoisonError::into_inner);
        if !rules.patterns.is_empty() || self.adblock().is_some() {
            return json!([{ "urlPattern": "*", "requestStage": "Request" }]);
        }

        let patterns: Vec<Value> = rules
            .types
            .iter()
            .flat_map(|kind| cdp_resource_types(*kind))
            .map(|name| json!({ "resourceType": name, "requestStage": "Request" }))
            .collect();

        Value::Array(patterns)
    }

    pub(crate) fn intercepting(&self) -> bool {
        !self
            .rules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
            || self.adblock().is_some()
    }

    fn effective_content<T>(&self, read: impl Fn(&ContentOptions) -> T) -> Option<T> {
        let content = self.content.lock().unwrap_or_else(PoisonError::into_inner);
        match &*content {
            Some(options) => Some(read(options)),
            None => self
                .defaults
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .map(read),
        }
    }

    pub(crate) fn adblock(&self) -> Option<Adblock> {
        self.effective_content(|options| options.adblock.clone())
            .flatten()
    }

    pub(crate) fn dismiss_cookies(&self) -> bool {
        self.effective_content(|options| options.dismiss_cookies)
            .unwrap_or(false)
    }

    pub(super) fn set_content(&self, options: ContentOptions) {
        *self.content.lock().unwrap_or_else(PoisonError::into_inner) = Some(options);
    }

    // Whether the Tab follows the defaults, i.e. whether they are now live for it.
    pub(crate) fn set_defaults(&self, options: Option<ContentOptions>) -> bool {
        *self.defaults.lock().unwrap_or_else(PoisonError::into_inner) = options;

        self.content
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
    }

    pub(super) fn consent_registered(&self, session_id: &SessionId) -> bool {
        self.consent_scripts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|(session, _)| session == session_id)
    }

    pub(super) fn add_consent_script(&self, session_id: &SessionId, identifier: String) {
        self.consent_scripts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((session_id.clone(), identifier));
    }

    pub(super) fn take_consent_scripts(&self) -> Vec<(SessionId, String)> {
        std::mem::take(
            &mut *self
                .consent_scripts
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    // Page and iframe sessions: the ones that own frames. Workers have none.
    pub(super) fn frame_sessions(&self) -> Vec<SessionId> {
        let mut sessions: Vec<SessionId> = Vec::new();
        for frame in self.tree().frames.values() {
            if !sessions.contains(&frame.session_id) {
                sessions.push(frame.session_id.clone());
            }
        }

        sessions
    }

    // The value and the forms a page or a URL shows it in: quoted in a snapshot, escaped in HTML,
    // percent-encoded in a URL.
    pub fn add_secret(&self, value: &str) {
        if value.is_empty() {
            return;
        }
        self.holds_secret.store(true, Ordering::Relaxed);

        let mut secrets = self.secrets.lock().unwrap_or_else(PoisonError::into_inner);
        for form in forms(value) {
            if !form.is_empty() && !secrets.iter().any(|secret| secret.0 == form) {
                secrets.push(Secret(form));
            }
        }
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.0.len()));
    }

    pub(super) fn lock_evaluate(&self) {
        self.evaluate_locks.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn unlock_evaluate(&self) {
        self.evaluate_locks.fetch_sub(1, Ordering::Relaxed);
    }

    pub(crate) fn evaluate_locked(&self) -> bool {
        self.evaluate_locks.load(Ordering::Relaxed) > 0
    }

    pub(super) fn navigations(&self) -> watch::Sender<u64> {
        self.navigations.clone()
    }

    pub(crate) fn holds_secret(&self) -> bool {
        self.holds_secret.load(Ordering::Relaxed)
    }

    // Borrowed unless a secret occurs: the common case allocates nothing.
    pub fn redact<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let secrets = self.secrets.lock().unwrap_or_else(PoisonError::into_inner);
        if !secrets.iter().any(|secret| text.contains(&secret.0)) {
            return Cow::Borrowed(text);
        }

        // One pass: the replacement text is never rescanned, so a secret that happens to be a
        // piece of «redacted» cannot mangle it.
        let mut redacted = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(next) = rest.chars().next() {
            match secrets.iter().find(|secret| rest.starts_with(&secret.0)) {
                Some(secret) => {
                    redacted.push_str(REDACTED);
                    rest = &rest[secret.0.len()..];
                }
                None => {
                    redacted.push(next);
                    rest = &rest[next.len_utf8()..];
                }
            }
        }

        Cow::Owned(redacted)
    }

    pub(crate) fn redact_owned(&self, text: String) -> String {
        let redacted = match self.redact(&text) {
            Cow::Owned(redacted) => Some(redacted),
            Cow::Borrowed(_) => None,
        };

        redacted.unwrap_or(text)
    }

    pub(crate) fn has_secrets(&self) -> bool {
        !self
            .secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    // Every string in the JSON value, object keys included.
    pub(crate) fn redact_value(&self, value: &mut Value) {
        if !self.has_secrets() {
            return;
        }
        match value {
            Value::String(text) => *text = self.redact_owned(std::mem::take(text)),
            Value::Array(items) => items.iter_mut().for_each(|item| self.redact_value(item)),
            Value::Object(fields) => {
                for (key, mut field) in std::mem::take(fields) {
                    self.redact_value(&mut field);
                    fields.insert(self.redact_owned(key), field);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn verdict(&self, request: &PausedRequest) -> Verdict {
        verdict(
            &self.rules.lock().unwrap_or_else(PoisonError::into_inner),
            request,
        )
    }

    pub(crate) fn add_session(&self, session_id: &SessionId) {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(session_id.clone());
    }

    pub(crate) fn sessions(&self) -> Vec<SessionId> {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // True once no request has been in flight for `quiet`.
    pub(crate) fn network_idle(&self, quiet: Duration) -> bool {
        self.network().idle(quiet)
    }

    // For an engine that reports no network-idle event of its own: the rest of `timeout`, counted
    // from `started`, is spent waiting for the tracked requests to go quiet.
    pub(crate) async fn wait_network_idle(
        &self,
        started: Instant,
        timeout: Duration,
    ) -> Result<(), Error> {
        let deadline = started + timeout;

        while !self.network_idle(NETWORK_QUIET) {
            if Instant::now() >= deadline {
                return Err(Error::Timeout {
                    after_ms: millis(timeout),
                });
            }
            tokio::time::sleep(NETWORK_POLL).await;
        }

        Ok(())
    }

    fn tree(&self) -> MutexGuard<'_, FrameTree> {
        self.frames.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn main_frame(&self) -> String {
        self.tree().main.clone()
    }

    pub(crate) fn frame(&self, frame_id: &str) -> Option<Frame> {
        self.tree().frames.get(frame_id).cloned()
    }

    pub(crate) fn frame_id_by_seq(&self, frame_seq: u32) -> Option<String> {
        self.tree()
            .frames
            .iter()
            .find(|(_, frame)| frame.frame_seq == frame_seq)
            .map(|(id, _)| id.clone())
    }

    pub(crate) fn local_root(&self, frame_id: &str) -> String {
        self.tree().local_root(frame_id)
    }

    pub(crate) fn is_main(&self, frame_id: &str) -> bool {
        self.tree().main == frame_id
    }

    pub(crate) fn owns_frame(&self, frame_id: &str) -> bool {
        self.tree().frames.contains_key(frame_id)
    }

    pub(crate) fn adopt(
        &self,
        frame_id: &str,
        session_id: &SessionId,
        parent: Option<&str>,
    ) -> bool {
        self.tree().adopt(frame_id, session_id, parent)
    }

    pub(crate) fn seed(&self, session_id: &SessionId, tree: &Value) -> Vec<String> {
        let ids = self.tree().seed(session_id, tree);
        self.changed.notify_waiters();

        ids
    }

    pub(crate) fn apply(&self, session_id: &SessionId, method: &str, params: &Value) {
        let main_navigated = match method {
            "Page.frameNavigated" => params["frame"]["parentId"].is_null(),
            NAVIGATION_COMMITTED => params["frameId"]
                .as_str()
                .is_some_and(|frame_id| self.is_main(frame_id)),
            _ => false,
        };
        if main_navigated {
            self.holds_secret.store(false, Ordering::Relaxed);
            self.navigations.send_modify(|count| *count += 1);
        }

        match method {
            JS_DIALOG_OPENING | JS_DIALOG_CLOSED => self.apply_dialog(session_id, method, params),
            _ if Network::tracks(method) => self.network().apply(session_id, method, params),
            _ => {
                self.tree().apply(session_id, method, params);
                self.changed.notify_waiters();
            }
        }
    }

    pub(crate) fn session_detached(&self, session_id: &SessionId) {
        self.tree().session_detached(session_id);

        self.network().session_detached(session_id);
        self.consent_scripts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(session, _)| session != session_id);
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|session| session != session_id);
    }

    pub(crate) fn set_utility(&self, frame_id: &str, context_id: i64) {
        self.tree().set_utility(frame_id, context_id);
    }

    // Only while the frame still has the context the instance was built in.
    pub(crate) fn set_injected(&self, frame_id: &str, context_id: i64, object_id: &str) {
        if let Some(frame) = self.tree().frames.get_mut(frame_id)
            && frame.utility_context == Some(context_id)
        {
            frame.injected = Some(object_id.to_owned());
        }
    }

    pub(crate) fn clear_injected(&self, frame_id: &str) {
        if let Some(frame) = self.tree().frames.get_mut(frame_id) {
            frame.injected = None;
        }
    }
}

fn escaped_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }

    escaped
}

fn percent_encoded(value: &str, keep: &str, space: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b' ' => encoded.push_str(space),
            b if b.is_ascii_alphanumeric() || keep.as_bytes().contains(&b) => {
                encoded.push(char::from(b));
            }
            b => encoded.push_str(&format!("%{b:02X}")),
        }
    }

    encoded
}

fn forms(value: &str) -> Vec<String> {
    let json = serde_json::to_string(value).unwrap_or_default();
    let json = json
        .strip_prefix('"')
        .and_then(|json| json.strip_suffix('"'))
        .unwrap_or_default()
        .to_owned();

    vec![
        value.to_owned(),
        json,
        escaped_html(value),
        percent_encoded(value, "-._~", "%20"),
        // encodeURIComponent leaves these alone.
        percent_encoded(value, "-._~!'()*", "%20"),
        percent_encoded(value, "-._~", "+"),
    ]
}
