use std::time::Duration;

use base64::Engine;
use brr_cdp::SessionId;
use brr_core::{Error, WaitUntil};
use serde_json::{Value, json};
use tokio::time::Instant;
use tracing::debug;

use super::input::{
    KeyDescription, KeyInput, KeyKind, MouseInput, MouseKind, effective, invalid_key,
    key_description, modifiers_mask, parse_combo,
};
use super::state::NETWORK_QUIET;
use super::{
    ClickOptions, DEFAULT_EVALUATE_TIMEOUT, Element, Modifier, MouseButton, POLL_INTERVAL_MS, Page,
    RETRY_BACKOFF_MS, SELECTOR_FIELD, Session, Target, UploadFile, WaitCondition, value_of,
};
use crate::chromium::millis;
use crate::common::CONNECTION_CLOSED;

#[cfg(test)]
mod tests;

pub const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

const MIN_QUAD_AREA: f64 = 0.99;
const DRAG_STEPS: u32 = 5;
const MAX_REASON_CHARS: usize = 200;
const VALUE_FIELD: &str = "value";
const ARGUMENT_VALUE: &str = "value";
const NOT_CONNECTED: &str = "error:notconnected";
const DELETE_KEY: &str = "Delete";
const DIALOG_GONE: &str = "No dialog is showing";
// Errors of a browser or tab that is gone: retrying them only hides the cause.
const GONE: [&str; 2] = [CONNECTION_CLOSED, "Session with given id not found"];
const SCROLL_ALIGNMENTS: [&str; 3] = [
    r#"{"block":"end","inline":"end"}"#,
    r#"{"block":"center","inline":"center"}"#,
    r#"{"block":"start","inline":"start"}"#,
];

const STATES_FUNCTION: &str =
    "function(node, states) { return this.checkElementStates(node, states); }";
const ELEMENT_STATE_FUNCTION: &str =
    "function(node, state) { return this.elementState(node, state); }";
const HIT_FUNCTION: &str = "function(node, point) { return this.expectHitTarget(point, node); }";
const FILL_FUNCTION: &str = "function(node, value) { return this.fill(node, value); }";
// The origin is checked in the same call that sets the value, so no navigation can land between.
const FILL_PINNED_FUNCTION: &str = "function(node, value, origin) { if (node.ownerDocument.location.origin !== origin) return 'wrongorigin'; return this.fill(node, value); }";
const WRONG_ORIGIN: &str = "wrongorigin";
const ORIGIN_EXPRESSION: &str = "location.origin";
const FOCUS_FUNCTION: &str = "function(node) { return this.focusNode(node); }";
const SELECT_OPTIONS_FUNCTION: &str =
    "function(node, options) { return this.selectOptions(node, options); }";
const INPUT_FILES_FUNCTION: &str =
    "function(node, files) { return this.setInputFiles(node, files); }";
const VIEWPORT_CENTER: &str = "({ x: innerWidth / 2, y: innerHeight / 2 })";
const VIEWPORT_SIZE: &str = "({ x: innerWidth, y: innerHeight })";
const SCROLL_FUNCTION: &str =
    "function(node, options) { node.scrollIntoView(options); return 'done'; }";
const IFRAME_STYLE_FUNCTION: &str = "function(node) { return this.describeIFrameStyle(node); }";
const TRANSFORMED: &str = "transformed";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Point {
    pub x: f64,
    pub y: f64,
}

// Playwright's waits between attempts, then a steady poll.
pub(crate) fn retry_wait(attempt: usize) -> Duration {
    Duration::from_millis(
        RETRY_BACKOFF_MS
            .get(attempt)
            .copied()
            .unwrap_or(POLL_INTERVAL_MS),
    )
}

fn quad_area(quad: &[f64; 8]) -> f64 {
    let mut area = 0.0;
    for index in 0..4 {
        let next = (index + 1) % 4;
        area += (quad[index * 2] * quad[next * 2 + 1] - quad[next * 2] * quad[index * 2 + 1]) / 2.0;
    }

    area.abs()
}

fn quad_center(quad: &[f64; 8]) -> Point {
    Point {
        x: (quad[0] + quad[2] + quad[4] + quad[6]) / 4.0,
        y: (quad[1] + quad[3] + quad[5] + quad[7]) / 4.0,
    }
}

// The first quad with visible area, as the center of what is left of it inside the viewport,
// after the offsets of every iframe above it.
pub(crate) fn clickable_point(
    quads: &[[f64; 8]],
    offsets: &[Point],
    viewport: &Point,
) -> Option<Point> {
    let shift_x: f64 = offsets.iter().map(|offset| offset.x).sum();
    let shift_y: f64 = offsets.iter().map(|offset| offset.y).sum();

    quads.iter().find_map(|quad| {
        let mut clipped = *quad;
        for pair in clipped.chunks_exact_mut(2) {
            pair[0] = (pair[0] + shift_x).clamp(0.0, viewport.x);
            pair[1] = (pair[1] + shift_y).clamp(0.0, viewport.y);
        }

        (quad_area(&clipped) >= MIN_QUAD_AREA).then(|| quad_center(&clipped))
    })
}

// `*` matches any run of characters, everything else is literal.
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return text.is_empty();
    };
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let mut parts: Vec<&str> = parts.collect();
    let Some(last) = parts.pop() else {
        return rest.is_empty();
    };

    for part in parts {
        let Some(found) = rest.find(part) else {
            return false;
        };
        rest = &rest[found + part.len()..];
    }

    rest.ends_with(last)
}

fn mouse_params(
    kind: &str,
    point: &Point,
    button: MouseButton,
    buttons: u32,
    modifiers: u32,
    click_count: u32,
) -> Value {
    json!({
        "type": kind,
        "x": point.x,
        "y": point.y,
        "button": button.name(),
        "buttons": buttons,
        "modifiers": modifiers,
        "clickCount": click_count,
    })
}

fn key_params(down: bool, key: &KeyDescription, modifiers: u32, auto_repeat: bool) -> Value {
    if !down {
        return json!({
            "type": "keyUp",
            "modifiers": modifiers,
            "key": key.key,
            "windowsVirtualKeyCode": key.key_code_without_location,
            "code": key.code,
            "location": key.location,
        });
    }

    json!({
        "type": if key.text.is_empty() { "rawKeyDown" } else { "keyDown" },
        "modifiers": modifiers,
        "windowsVirtualKeyCode": key.key_code_without_location,
        "code": key.code,
        "key": key.key,
        "text": key.text,
        "unmodifiedText": key.text,
        "autoRepeat": auto_repeat,
        "location": key.location,
        "isKeypad": key.is_keypad(),
    })
}

// An error thrown inside the page reads "Error: message\n    at ..."; the first line is the message.
fn thrown_message(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();

    line.strip_prefix("Error: ").unwrap_or(line).to_owned()
}

enum Step<T> {
    Done(T),
    Retry(String),
    // Nothing matched yet; still not there at the deadline means ElementNotFound.
    Missing,
}

// What a navigation or a vanished node causes; a browser that is gone is not transient.
fn is_transient(error: &Error) -> bool {
    match error {
        Error::Internal { message } => !GONE.iter().any(|gone| message.contains(gone)),
        Error::EvaluationFailed { .. } => true,
        _ => false,
    }
}

// A DOM call that fails because the node went away is a reason to try again, not an error.
macro_rules! soft {
    ($call:expr) => {
        match $call {
            Ok(value) => value,
            Err(error) if is_transient(&error) => return Ok(Step::Retry(error.to_string())),
            Err(error) => return Err(error),
        }
    };
}

struct Located {
    element: Element,
    // Top-level viewport coordinates.
    point: Point,
}

impl Page {
    // Playwright's retry waits; StaleRef and InvalidArgument end the loop at once.
    // The attempt returns a `Send` future: an `AsyncFnMut` one cannot be proven `Send` by callers.
    async fn retry<T, F>(
        &self,
        timeout: Duration,
        mut attempt: impl FnMut(usize) -> F,
    ) -> Result<T, Error>
    where
        F: Future<Output = Result<Step<T>, Error>> + Send,
    {
        let deadline = Instant::now() + timeout;
        let timed_out = || Error::Timeout {
            after_ms: millis(timeout),
        };
        let mut tries = 0;
        let mut missing = false;

        loop {
            let step = match tokio::time::timeout_at(deadline, attempt(tries)).await {
                // An attempt cut off at the deadline right after one that found nothing is
                // still a missing element, not a slow one.
                Err(_) if missing => return Err(Error::ElementNotFound),
                Err(_) => return Err(timed_out()),
                Ok(Err(Error::ElementNotFound)) => Step::Missing,
                Ok(result) => result?,
            };
            missing = match step {
                Step::Done(value) => return Ok(value),
                Step::Retry(reason) => {
                    debug!(
                        reason = reason.chars().take(MAX_REASON_CHARS).collect::<String>(),
                        "action not ready"
                    );
                    false
                }
                Step::Missing => true,
            };

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(if missing {
                    Error::ElementNotFound
                } else {
                    timed_out()
                });
            }
            tokio::time::sleep(retry_wait(tries).min(remaining)).await;
            tries += 1;
        }
    }

    pub(super) async fn call_element(
        &self,
        element: &Element,
        declaration: &str,
        arguments: impl IntoIterator<Item = Value>,
    ) -> Result<Value, Error> {
        let mut call = vec![json!({ "objectId": element.object_id })];
        // Moved into the argument, not copied: an upload's value is megabytes.
        call.extend(arguments.into_iter().map(|value| {
            let mut argument = serde_json::Map::new();
            argument.insert(ARGUMENT_VALUE.to_owned(), value);

            Value::Object(argument)
        }));
        let (_, mut remote) = self
            .call_injected_args(&element.frame_id, declaration, call, true)
            .await?;

        Ok(value_of(&mut remote))
    }

    // Fire and forget: a page stalled on a dialog would hold the caller until the call times out.
    pub(super) fn release(&self, element: &Element) {
        let context_id = self
            .state
            .frame(&element.frame_id)
            .and_then(|frame| frame.utility_context);

        self.session.release(element, context_id);
    }

    async fn content_quads(&self, element: &Element) -> Result<Vec<[f64; 8]>, Error> {
        self.session.content_quads(element).await
    }

    // Where the content of `frame_id` starts in the top-level viewport, one offset per iframe
    // up the chain. Quads of a frame in another process are relative to that process's root.
    pub(super) async fn frame_offsets(&self, frame_id: &str) -> Result<Vec<Point>, Error> {
        let mut offsets = Vec::new();
        let mut current = frame_id.to_owned();

        while let Some(parent_id) = self.state.frame(&current).and_then(|frame| frame.parent) {
            if self.state.frame(&parent_id).is_none() {
                break;
            }

            offsets.push(
                self.session
                    .frame_origin(&current, &parent_id, &self.state)
                    .await?,
            );

            current = self.state.local_root(&parent_id);
        }

        Ok(offsets)
    }

    // The iframe element that holds `child`, resolved in the parent's utility world.
    async fn frame_element(&self, child: &str, parent: &str) -> Result<Element, Error> {
        let injected = self.injected(parent).await?;
        let context_id = self
            .state
            .frame(parent)
            .and_then(|frame| frame.utility_context);

        self.session
            .frame_element(child, parent, &self.state, &injected.session_id, context_id)
            .await
    }

    async fn origin(&self, frame_id: &str) -> Result<Point, Error> {
        let offsets = self.frame_offsets(frame_id).await?;

        Ok(Point {
            x: offsets.iter().map(|offset| offset.x).sum(),
            y: offsets.iter().map(|offset| offset.y).sum(),
        })
    }

    // Playwright's _checkFrameIsHitTarget: every iframe above the element must receive the
    // point, and then the element must. A transformed iframe cannot be translated, so the check
    // is skipped and the click lands where the quads say.
    async fn hit_target(&self, element: &Element, point: &Point) -> Result<Step<()>, Error> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut current = element.frame_id.clone();
        while let Some(parent) = self.state.frame(&current).and_then(|frame| frame.parent) {
            pairs.push((current, parent.clone()));
            current = parent;
        }

        let mut iframes: Vec<Element> = Vec::new();
        let chain = self.hit_chain(&pairs, point, &mut iframes).await;
        for iframe in &iframes {
            self.release(iframe);
        }
        match chain? {
            Step::Done(true) => {}
            Step::Done(false) => return Ok(Step::Done(())),
            Step::Retry(reason) => return Ok(Step::Retry(reason)),
            Step::Missing => return Ok(Step::Missing),
        }

        let origin = soft!(self.origin(&element.frame_id).await);
        let in_frame = json!({ "x": point.x - origin.x, "y": point.y - origin.y });
        let hit = soft!(self.call_element(element, HIT_FUNCTION, [in_frame]).await);

        Ok(match hit["hitTargetDescription"].as_str() {
            Some(description) => Step::Retry(format!("{description} intercepts pointer events")),
            None => Step::Done(()),
        })
    }

    // Top frame first. Done(true): every iframe is hit; Done(false): one is transformed.
    async fn hit_chain(
        &self,
        pairs: &[(String, String)],
        point: &Point,
        iframes: &mut Vec<Element>,
    ) -> Result<Step<bool>, Error> {
        for (child, parent) in pairs.iter().rev() {
            match self.hit_iframe(child, parent, point, iframes).await? {
                Step::Done(true) => {}
                other => return Ok(other),
            }
        }

        Ok(Step::Done(true))
    }

    async fn hit_iframe(
        &self,
        child: &str,
        parent: &str,
        point: &Point,
        iframes: &mut Vec<Element>,
    ) -> Result<Step<bool>, Error> {
        let iframe = soft!(self.frame_element(child, parent).await);
        iframes.push(iframe.clone());

        let style = soft!(self.call_element(&iframe, IFRAME_STYLE_FUNCTION, []).await);
        if style == TRANSFORMED {
            return Ok(Step::Done(false));
        }
        if style == NOT_CONNECTED {
            return Ok(Step::Retry("an iframe is not attached".to_owned()));
        }

        let origin = soft!(self.origin(parent).await);
        let in_parent = json!({ "x": point.x - origin.x, "y": point.y - origin.y });
        let hit = soft!(self.call_element(&iframe, HIT_FUNCTION, [in_parent]).await);

        Ok(match hit["hitTargetDescription"].as_str() {
            Some(description) => Step::Retry(format!("{description} intercepts pointer events")),
            None => Step::Done(true),
        })
    }

    // Steps 2 to 6 of a pointer action on a resolved element.
    async fn locate_element(
        &self,
        element: &Element,
        target: &Target,
        wait_enabled: bool,
        attempt: usize,
    ) -> Result<Step<Point>, Error> {
        let states = if wait_enabled {
            json!(["visible", "enabled", "stable"])
        } else {
            json!(["visible", "stable"])
        };
        let state = soft!(self.call_element(element, STATES_FUNCTION, [states]).await);
        if state == NOT_CONNECTED {
            return match target {
                Target::Ref(_) => Err(Error::StaleRef),
                Target::Selector(_) => Ok(Step::Retry("element is not attached".to_owned())),
            };
        }
        if let Some(missing) = state["missingState"].as_str() {
            return Ok(Step::Retry(format!("element is not {missing}")));
        }

        // Retries cycle the alignment: an element under a sticky header counts as in view for
        // the protocol scroll, so only another alignment reveals it.
        match attempt
            .checked_rem(SCROLL_ALIGNMENTS.len() + 1)
            .and_then(|slot| slot.checked_sub(1))
            .and_then(|slot| SCROLL_ALIGNMENTS.get(slot))
        {
            None => {
                soft!(self.session.scroll_into_view(element).await);
            }
            Some(alignment) => {
                let options: Value = serde_json::from_str(alignment).unwrap_or(Value::Null);
                soft!(self.call_element(element, SCROLL_FUNCTION, [options]).await);
            }
        }
        let quads = soft!(self.content_quads(element).await);

        let root = self.state.local_root(&element.frame_id);
        let root_offsets = soft!(self.frame_offsets(&root).await);
        let size = soft!(self.main_viewport().await);
        let Some(point) = clickable_point(&quads, &root_offsets, &size) else {
            return Ok(Step::Retry("element has no visible area".to_owned()));
        };

        Ok(match self.hit_target(element, &point).await? {
            Step::Done(()) => Step::Done(point),
            Step::Retry(reason) => Step::Retry(reason),
            Step::Missing => Step::Missing,
        })
    }

    async fn main_viewport(&self) -> Result<Point, Error> {
        let size = self.evaluate_utility(VIEWPORT_SIZE).await?;

        Ok(Point {
            x: size["x"].as_f64().unwrap_or_default(),
            y: size["y"].as_f64().unwrap_or_default(),
        })
    }

    async fn locate_once(
        &self,
        target: &Target,
        wait_enabled: bool,
        attempt: usize,
    ) -> Result<Step<Located>, Error> {
        let element = self.resolve(target, Duration::ZERO).await?;

        match self
            .locate_element(&element, target, wait_enabled, attempt)
            .await
        {
            Ok(Step::Done(point)) => Ok(Step::Done(Located { element, point })),
            other => {
                self.release(&element);

                other.map(|step| match step {
                    Step::Retry(reason) => Step::Retry(reason),
                    _ => Step::Missing,
                })
            }
        }
    }

    async fn locate(
        &self,
        target: &Target,
        wait_enabled: bool,
        timeout: Duration,
    ) -> Result<Located, Error> {
        self.retry(timeout, move |attempt| async move {
            self.locate_once(target, wait_enabled, attempt).await
        })
        .await
    }

    async fn modifier_key(
        &self,
        modifier: Modifier,
        down: bool,
        held: &[Modifier],
    ) -> Result<(), Error> {
        let name = modifier.key_name();
        let description =
            key_description(name).ok_or_else(|| invalid_key(format!("unknown key \"{name}\"")))?;

        self.session
            .dispatch_key(key_params(down, description, modifiers_mask(held), false))
            .await
    }

    // The modifiers are released whatever happened in between: a stuck Shift breaks every
    // later tool.
    async fn with_modifiers(
        &self,
        modifiers: &[Modifier],
        action: impl AsyncFnOnce(&[Modifier]) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut held: Vec<Modifier> = Vec::new();
        let mut result = Ok(());

        for modifier in modifiers {
            held.push(*modifier);
            result = self.modifier_key(*modifier, true, &held).await;
            if result.is_err() {
                break;
            }
        }
        if result.is_ok() {
            result = action(&held).await;
        }
        while let Some(modifier) = held.pop() {
            let released = self.modifier_key(modifier, false, &held).await;
            result = result.and(released);
        }

        result
    }

    // Input runs in its own task, so a cancelled call still releases the keys it pressed. A
    // dialog opened by the input stalls the renderer's reply to it; the call returns then, and
    // the task finishes once the dialog is answered.
    async fn run_input(
        &self,
        work: impl Future<Output = Result<(), Error>> + Send + 'static,
    ) -> Result<(), Error> {
        let mut dialog = self.state.dialog();
        let mut task = tokio::spawn(work);

        loop {
            tokio::select! {
                result = &mut task => {
                    return result.map_err(|error| Error::Internal {
                        message: format!("input task failed: {error}"),
                    })?;
                }
                Ok(()) = dialog.changed() => {
                    if dialog.borrow().is_some() {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn click_at(&self, point: &Point, options: &ClickOptions) -> Result<(), Error> {
        let page = self.clone();
        let point = point.clone();
        let options = options.clone();

        self.run_input(async move { page.click_sequence(&point, &options).await })
            .await
    }

    async fn click_sequence(&self, point: &Point, options: &ClickOptions) -> Result<(), Error> {
        self.with_modifiers(&options.modifiers, async |_| {
            let mask = modifiers_mask(&options.modifiers);
            let button = options.button;
            let move_event = mouse_params("mouseMoved", point, MouseButton::None, 0, mask, 0);
            self.session.dispatch_mouse(move_event).await?;

            for count in 1..=options.click_count.max(1) {
                let pressed =
                    mouse_params("mousePressed", point, button, button.mask(), mask, count);
                self.session.dispatch_mouse(pressed).await?;
                let released = mouse_params("mouseReleased", point, button, 0, mask, count);
                self.session.dispatch_mouse(released).await?;
            }

            Ok(())
        })
        .await
    }

    pub async fn click(
        &self,
        target: &Target,
        options: &ClickOptions,
        timeout: Duration,
    ) -> Result<(), Error> {
        let located = self.locate(target, true, timeout).await?;
        let result = self.click_at(&located.point, options).await;
        self.release(&located.element);

        result
    }

    pub async fn hover(&self, target: &Target, timeout: Duration) -> Result<(), Error> {
        let located = self.locate(target, false, timeout).await?;
        let event = mouse_params("mouseMoved", &located.point, MouseButton::None, 0, 0, 0);
        let result = self.session.dispatch_mouse(event).await;
        self.release(&located.element);

        result
    }

    // Actionable: visible, enabled and editable; the injected `fill` then picks the way to set it.
    pub async fn fill(&self, target: &Target, value: &str, timeout: Duration) -> Result<(), Error> {
        self.fill_in(target, value, timeout, None).await
    }

    // For a secret the caller vetted on the page at `origin`: the value is only set while the
    // element's document has that origin, and the fill ends with StaleRef otherwise.
    pub async fn fill_pinned(
        &self,
        target: &Target,
        value: &str,
        timeout: Duration,
        origin: &str,
    ) -> Result<(), Error> {
        self.fill_in(target, value, timeout, Some(origin)).await
    }

    async fn fill_in(
        &self,
        target: &Target,
        value: &str,
        timeout: Duration,
        pinned: Option<&str>,
    ) -> Result<(), Error> {
        let needs_input = self
            .retry(timeout, move |_| async move {
                let element = self.resolve(target, Duration::ZERO).await?;
                let step = self.fill_once(&element, target, value, pinned).await;
                self.release(&element);

                step
            })
            .await?;

        if !needs_input {
            return Ok(());
        }
        if value.is_empty() {
            return self.press_key(None, DELETE_KEY, timeout).await;
        }
        // Typing goes through the browser's input pipeline, so the check cannot share a call with
        // it; this is the smallest gap left.
        if let Some(origin) = pinned {
            let current = self
                .evaluate_internal(ORIGIN_EXPRESSION, DEFAULT_EVALUATE_TIMEOUT)
                .await?;
            if current != origin {
                return Err(Error::StaleRef);
            }
        }

        self.insert_text(value).await
    }

    // Ok(true): the value still has to be typed in.
    async fn fill_once(
        &self,
        element: &Element,
        target: &Target,
        value: &str,
        pinned: Option<&str>,
    ) -> Result<Step<bool>, Error> {
        let invalid = |field: &'static str| {
            move |error: Error| match error {
                Error::EvaluationFailed { message } => Error::InvalidArgument {
                    field: field.to_owned(),
                    message: thrown_message(&message),
                },
                other => other,
            }
        };

        let states = json!(["visible", "enabled", "editable"]);
        let state = soft!(
            self.call_element(element, STATES_FUNCTION, [states])
                .await
                .map_err(invalid(SELECTOR_FIELD))
        );
        if state == NOT_CONNECTED {
            return Ok(Self::detached(target));
        }
        if let Some(missing) = state["missingState"].as_str() {
            return Ok(Step::Retry(format!("element is not {missing}")));
        }

        let filled = soft!(
            match pinned {
                Some(origin) => {
                    self.call_element(element, FILL_PINNED_FUNCTION, [json!(value), json!(origin)])
                        .await
                }
                None => {
                    self.call_element(element, FILL_FUNCTION, [json!(value)])
                        .await
                }
            }
            .map_err(invalid(VALUE_FIELD))
        );

        Ok(match filled.as_str() {
            Some(WRONG_ORIGIN) => return Err(Error::StaleRef),
            Some("needsinput") => Step::Done(true),
            Some("done") => Step::Done(false),
            _ => Self::detached(target),
        })
    }

    fn detached<T>(target: &Target) -> Step<T> {
        match target {
            Target::Ref(_) => Step::Retry("element is not attached".to_owned()),
            Target::Selector(_) => Step::Missing,
        }
    }

    async fn focus(&self, target: &Target, timeout: Duration) -> Result<(), Error> {
        let element = self.resolve(target, timeout).await?;
        let result = self.call_element(&element, FOCUS_FUNCTION, []).await;
        self.release(&element);

        if result? == NOT_CONNECTED {
            return Err(Error::StaleRef);
        }

        Ok(())
    }

    async fn press_description(
        &self,
        key: &KeyDescription,
        held: &[Modifier],
    ) -> Result<(), Error> {
        let key = effective(key, held);
        let mask = modifiers_mask(held);

        self.session
            .dispatch_key(key_params(true, &key, mask, false))
            .await?;
        self.session
            .dispatch_key(key_params(false, &key, mask, false))
            .await
    }

    pub async fn type_text(
        &self,
        target: Option<&Target>,
        text: &str,
        delay: Duration,
        timeout: Duration,
    ) -> Result<(), Error> {
        if let Some(target) = target {
            self.focus(target, timeout).await?;
        }

        let page = self.clone();
        let text = text.to_owned();

        self.run_input(async move {
            for (index, character) in text.chars().enumerate() {
                if index > 0 && !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                let character = character.to_string();
                match key_description(&character) {
                    Some(key) => page.press_description(key, &[]).await?,
                    None => page.session.insert_text(&character).await?,
                }
            }

            Ok(())
        })
        .await
    }

    pub async fn press_key(
        &self,
        target: Option<&Target>,
        key: &str,
        timeout: Duration,
    ) -> Result<(), Error> {
        let combo = parse_combo(key)?;
        if let Some(target) = target {
            self.focus(target, timeout).await?;
        }

        let page = self.clone();
        self.run_input(async move {
            page.with_modifiers(&combo.modifiers, async |held| {
                page.press_description(combo.key, held).await
            })
            .await
        })
        .await
    }

    pub async fn select_option(
        &self,
        target: &Target,
        values: &[String],
        timeout: Duration,
    ) -> Result<Vec<String>, Error> {
        let options: Vec<Value> = values
            .iter()
            .map(|value| json!({ "valueOrLabel": value }))
            .collect();

        let options = &options;

        self.retry(timeout, move |_| async move {
            let element = self.resolve(target, Duration::ZERO).await?;
            let step = self.select_once(&element, target, options).await;
            self.release(&element);

            step
        })
        .await
    }

    async fn select_once(
        &self,
        element: &Element,
        target: &Target,
        options: &[Value],
    ) -> Result<Step<Vec<String>>, Error> {
        let states = json!(["visible", "enabled"]);
        let state = soft!(self.call_element(element, STATES_FUNCTION, [states]).await);
        if state == NOT_CONNECTED {
            return Ok(Self::detached(target));
        }
        if let Some(missing) = state["missingState"].as_str() {
            return Ok(Step::Retry(format!("element is not {missing}")));
        }

        let selected = soft!(
            self.call_element(element, SELECT_OPTIONS_FUNCTION, [json!(options)])
                .await
        );

        Ok(match selected {
            Value::Array(values) => Step::Done(
                values
                    .into_iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect(),
            ),
            Value::String(error) if error == "error:optionsnotfound" => Step::Missing,
            Value::String(error) => Step::Retry(error),
            _ => Step::Retry("unexpected answer".to_owned()),
        })
    }

    async fn is_checked(&self, target: &Target, timeout: Duration) -> Result<bool, Error> {
        let element = self.resolve(target, timeout).await?;
        let state = self
            .call_element(&element, ELEMENT_STATE_FUNCTION, [json!("checked")])
            .await;
        self.release(&element);

        match state {
            Ok(state) => Ok(state["received"] == "checked"),
            Err(Error::EvaluationFailed { message }) => Err(Error::InvalidArgument {
                field: SELECTOR_FIELD.to_owned(),
                message: thrown_message(&message),
            }),
            Err(error) => Err(error),
        }
    }

    pub async fn set_checked(
        &self,
        target: &Target,
        checked: bool,
        timeout: Duration,
    ) -> Result<(), Error> {
        if self.is_checked(target, timeout).await? == checked {
            return Ok(());
        }

        let options = ClickOptions {
            button: MouseButton::Left,
            modifiers: Vec::new(),
            click_count: 1,
        };
        self.click(target, &options, timeout).await?;

        if self.is_checked(target, timeout).await? != checked {
            return Err(Error::InvalidArgument {
                field: SELECTOR_FIELD.to_owned(),
                message: "clicking did not change the checked state".to_owned(),
            });
        }

        Ok(())
    }

    pub async fn drag(&self, from: &Target, to: &Target, timeout: Duration) -> Result<(), Error> {
        let start = self.locate(from, true, timeout).await?;
        let destination = self.locate(to, true, timeout).await;
        self.release(&start.element);
        let destination = destination?;
        self.release(&destination.element);

        let page = self.clone();
        self.run_input(async move { page.drag_to(&start.point, &destination.point).await })
            .await
    }

    // The button is released wherever the drag ends.
    async fn drag_to(&self, from: &Point, to: &Point) -> Result<(), Error> {
        let left = MouseButton::Left;
        let moved = mouse_params("mouseMoved", from, MouseButton::None, 0, 0, 0);
        self.session.dispatch_mouse(moved).await?;
        let pressed = mouse_params("mousePressed", from, left, left.mask(), 0, 1);
        self.session.dispatch_mouse(pressed).await?;

        let mut result = Ok(());
        for step in 1..=DRAG_STEPS {
            let fraction = f64::from(step) / f64::from(DRAG_STEPS);
            let point = Point {
                x: from.x + (to.x - from.x) * fraction,
                y: from.y + (to.y - from.y) * fraction,
            };
            result = self
                .session
                .dispatch_mouse(mouse_params("mouseMoved", &point, left, left.mask(), 0, 0))
                .await;
            if result.is_err() {
                break;
            }
        }
        let released = mouse_params("mouseReleased", to, left, 0, 0, 1);

        result.and(self.session.dispatch_mouse(released).await)
    }

    pub async fn upload(
        &self,
        target: &Target,
        files: &[UploadFile],
        timeout: Duration,
    ) -> Result<(), Error> {
        let total: usize = files.iter().map(|file| file.data.len()).sum();
        if total > MAX_UPLOAD_BYTES {
            return Err(Error::InvalidArgument {
                field: "files".to_owned(),
                message: format!("uploads are limited to {MAX_UPLOAD_BYTES} bytes in total"),
            });
        }
        let payloads: Vec<Value> = files
            .iter()
            .map(|file| {
                let mut payload = serde_json::Map::new();
                payload.insert("name".to_owned(), Value::String(file.name.clone()));
                payload.insert("mimeType".to_owned(), Value::String(file.mime.clone()));
                payload.insert(
                    "buffer".to_owned(),
                    Value::String(base64::engine::general_purpose::STANDARD.encode(&file.data)),
                );

                Value::Object(payload)
            })
            .collect();

        let element = self.resolve(target, timeout).await?;
        let result = self
            .call_element(&element, INPUT_FILES_FUNCTION, [Value::Array(payloads)])
            .await;
        self.release(&element);

        match result? {
            Value::String(message) => Err(Error::InvalidArgument {
                field: SELECTOR_FIELD.to_owned(),
                message,
            }),
            _ => Ok(()),
        }
    }

    pub async fn scroll_by(&self, dx: f64, dy: f64) -> Result<(), Error> {
        let center = self.evaluate_utility(VIEWPORT_CENTER).await?;
        let point = Point {
            x: center["x"].as_f64().unwrap_or_default(),
            y: center["y"].as_f64().unwrap_or_default(),
        };
        let mut params = mouse_params("mouseWheel", &point, MouseButton::None, 0, 0, 0);
        params["deltaX"] = json!(dx);
        params["deltaY"] = json!(dy);

        self.session.dispatch_mouse(params).await
    }

    pub async fn scroll_into_view(&self, target: &Target, timeout: Duration) -> Result<(), Error> {
        let element = self.resolve(target, timeout).await?;
        let result = self.session.scroll_into_view(&element).await;
        self.release(&element);

        result
    }

    pub async fn wait_for(
        &self,
        condition: &WaitCondition,
        timeout: Duration,
    ) -> Result<(), Error> {
        self.retry(timeout, move |_| async move {
            let ready = match condition {
                WaitCondition::Selector(selector) => {
                    self.selector_visible(&Target::Selector(selector.clone()))
                        .await
                }
                WaitCondition::Text(text) => {
                    let expression =
                        format!("(document.body?.innerText ?? '').includes({})", json!(text));

                    self.evaluate_utility(&expression)
                        .await
                        .map(|found| found == json!(true))
                }
                WaitCondition::Url(pattern) => self.url().await.map(|url| {
                    url == *pattern || (pattern.contains('*') && glob_match(pattern, &url))
                }),
                WaitCondition::NetworkIdle => Ok(self.state.network_idle(NETWORK_QUIET)),
            };

            // Waiting out a navigation is what this is for: its errors mean "not yet".
            match ready {
                Ok(true) => Ok(Step::Done(())),
                Ok(false) => Ok(Step::Retry("condition not met".to_owned())),
                Err(error) if is_transient(&error) => Ok(Step::Retry(error.to_string())),
                Err(error) => Err(error),
            }
        })
        .await
    }

    pub async fn selector_visible(&self, target: &Target) -> Result<bool, Error> {
        let element = match self.resolve(target, Duration::ZERO).await {
            Ok(element) => element,
            Err(Error::ElementNotFound) => return Ok(false),
            Err(error) => return Err(error),
        };
        let state = self
            .call_element(&element, STATES_FUNCTION, [json!(["visible"])])
            .await;
        self.release(&element);

        Ok(state? == Value::Null)
    }

    pub async fn handle_dialog(
        &self,
        accept: bool,
        prompt_text: Option<&str>,
    ) -> Result<(), Error> {
        if self.state.dialog().borrow().is_none() {
            return Err(Error::NotFound {
                what: "open dialog".to_owned(),
            });
        }
        let session_id: SessionId = self
            .state
            .dialog_session()
            .unwrap_or_else(|| self.session.main_session().clone());

        // Answered by someone else since the check above.
        self.session
            .handle_dialog(&session_id, accept, prompt_text)
            .await
            .map_err(|error| match error {
                Error::Internal { message } if message.contains(DIALOG_GONE) => Error::NotFound {
                    what: "open dialog".to_owned(),
                },
                other => other,
            })
    }

    // false: there is no entry that way.
    pub async fn go_back(&self, wait_until: WaitUntil, timeout: Duration) -> Result<bool, Error> {
        self.history(-1, wait_until, timeout).await
    }

    pub async fn go_forward(
        &self,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<bool, Error> {
        self.history(1, wait_until, timeout).await
    }

    async fn history(
        &self,
        delta: i64,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<bool, Error> {
        let started = Instant::now();
        let main = self.state.main_frame();
        let moved = self
            .session
            .go_history(delta, &main, wait_until, timeout)
            .await?;
        if moved {
            self.settle_network(wait_until, started, timeout).await?;
        }

        Ok(moved)
    }

    pub async fn reload(&self, wait_until: WaitUntil, timeout: Duration) -> Result<(), Error> {
        let started = Instant::now();
        let main = self.state.main_frame();
        self.session.reload(&main, wait_until, timeout).await?;

        self.settle_network(wait_until, started, timeout).await
    }

    // Chromium reports network idle itself; Camoufox's load event is followed by the tracked
    // requests going quiet.
    async fn settle_network(
        &self,
        wait_until: WaitUntil,
        started: Instant,
        timeout: Duration,
    ) -> Result<(), Error> {
        if wait_until != WaitUntil::Networkidle || !matches!(self.session, Session::Camoufox(_)) {
            return Ok(());
        }

        self.state.wait_network_idle(started, timeout).await
    }

    pub async fn mouse(&self, input: &MouseInput) -> Result<(), Error> {
        let point = Point {
            x: input.x,
            y: input.y,
        };
        let params = match input.kind {
            MouseKind::Down => mouse_params(
                "mousePressed",
                &point,
                input.button,
                input.button.mask(),
                input.modifiers,
                input.click_count,
            ),
            MouseKind::Up => mouse_params(
                "mouseReleased",
                &point,
                input.button,
                0,
                input.modifiers,
                input.click_count,
            ),
            MouseKind::Move => mouse_params(
                "mouseMoved",
                &point,
                MouseButton::None,
                0,
                input.modifiers,
                0,
            ),
            MouseKind::Wheel => {
                let mut params = mouse_params(
                    "mouseWheel",
                    &point,
                    MouseButton::None,
                    0,
                    input.modifiers,
                    0,
                );
                params["deltaX"] = json!(input.delta_x);
                params["deltaY"] = json!(input.delta_y);

                params
            }
        };

        let session = self.session.clone();

        self.run_input(async move { session.dispatch_mouse(params).await })
            .await
    }

    pub async fn key(&self, input: &KeyInput) -> Result<(), Error> {
        let mut params = json!({
            "type": match input.kind {
                KeyKind::Down => "keyDown",
                KeyKind::Up => "keyUp",
            },
            "key": input.key,
            "code": input.code,
            "windowsVirtualKeyCode": input.key_code,
            "modifiers": input.modifiers,
        });
        if input.kind == KeyKind::Down && !input.text.is_empty() {
            params["text"] = json!(input.text);
        }

        let session = self.session.clone();

        self.run_input(async move { session.dispatch_key(params).await })
            .await
    }

    pub async fn insert_text(&self, text: &str) -> Result<(), Error> {
        let session = self.session.clone();
        let text = text.to_owned();

        self.run_input(async move { session.insert_text(&text).await })
            .await
    }
}
