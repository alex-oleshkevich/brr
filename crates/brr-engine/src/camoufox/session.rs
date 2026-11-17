use std::sync::{Arc, Mutex};
use std::time::Duration;

use brr_cdp::SessionId;
use brr_content::AUTOCONSENT_JS;
use brr_core::{Error, TargetId};
use brr_juggler::{Juggler, SessionId as JugglerSessionId};
use serde_json::{Value, json};

use crate::common::{gone, internal};
use crate::page::{Element, PageState, UTILITY_WORLD};

const CONTEXT_PREFIX: &str = "id-";
const UTILITY_WAIT: Duration = Duration::from_secs(5);
// Firefox's utility world sees the page's structuredClone, whose results it may not store in
// its own objects; autoconsent then falls back to its JSON copy.
const STRUCTURED_CLONE_SHIM: &str = "Object.defineProperty(globalThis, 'structuredClone', { value: undefined, configurable: true });\n";
const MAX_ERROR_CHARS: usize = 4096;
// Camoufox recognizes a main-world request by this text in the function and the prefix on the code.
const MAIN_WORLD_CALL: &str = "(utilityScript, ...args) => utilityScript.evaluate(...args)";
const MAIN_WORLD_PREFIX: &str = "mw:";

#[derive(Clone)]
pub(crate) struct PageSession {
    pub juggler: Juggler,
    // The id the shared page state knows this session by.
    pub session_id: SessionId,
    pub juggler_session: JugglerSessionId,
    pub target_id: TargetId,
    // None is the default browser context, which Juggler addresses by leaving the id out.
    pub context_id: Option<String>,
    // The open dialog's id; the shared dialog state only carries what a caller may read.
    pub dialog_id: Arc<Mutex<Option<String>>>,
}

// Juggler names a context `id-N`; the shared frame tree keeps the number.
fn context_number(id: &str) -> Option<i64> {
    id.strip_prefix(CONTEXT_PREFIX)?.parse().ok()
}

fn context_name(number: i64) -> String {
    format!("{CONTEXT_PREFIX}{number}")
}

// A Juggler event as the CDP-shaped event the shared frame tree and dialog state read. None for
// events the shared state does not track.
pub(super) fn translate(method: &str, params: &Value) -> Option<(&'static str, Value)> {
    Some(match method {
        "Runtime.executionContextCreated" => {
            let name = params["auxData"]["name"].as_str().unwrap_or_default();

            (
                "Runtime.executionContextCreated",
                json!({ "context": {
                    "id": context_number(params["executionContextId"].as_str()?)?,
                    "name": name,
                    "auxData": { "frameId": params["auxData"]["frameId"], "isDefault": name.is_empty() },
                } }),
            )
        }
        "Runtime.executionContextDestroyed" => (
            "Runtime.executionContextDestroyed",
            json!({ "executionContextId": context_number(params["executionContextId"].as_str()?)? }),
        ),
        "Runtime.executionContextsCleared" => ("Runtime.executionContextsCleared", Value::Null),
        "Page.dialogOpened" => (
            crate::page::JS_DIALOG_OPENING,
            json!({
                "type": params["type"],
                "message": params["message"],
                "defaultPrompt": params["defaultValue"],
            }),
        ),
        "Page.dialogClosed" => (crate::page::JS_DIALOG_CLOSED, Value::Null),
        "Page.frameAttached" => ("Page.frameAttached", params.clone()),
        "Page.frameDetached" => ("Page.frameDetached", params.clone()),
        "Page.navigationCommitted" => ("Page.navigationCommitted", params.clone()),
        _ => return None,
    })
}

// Every frame gets the utility world from the first moment, so its context event is awaited
// rather than created on demand.
// The call replaces the Tab's whole script list, so it is also how autoconsent is turned off.
pub(crate) async fn init_session(
    session: &PageSession,
    dismiss_cookies: bool,
) -> Result<(), Error> {
    let mut scripts = vec![json!({ "script": "", "worldName": UTILITY_WORLD })];
    if dismiss_cookies {
        scripts.push(
            json!({ "script": format!("{STRUCTURED_CLONE_SHIM}{AUTOCONSENT_JS}"), "worldName": UTILITY_WORLD }),
        );
    }
    session
        .call("Page.setInitScripts", json!({ "scripts": scripts }))
        .await?;

    Ok(())
}

pub(crate) async fn utility_context(state: &PageState, frame_id: &str) -> Result<i64, Error> {
    let found = tokio::time::timeout(UTILITY_WAIT, async {
        loop {
            let changed = state.changed.notified();
            if let Some(context_id) = state
                .frame(frame_id)
                .and_then(|frame| frame.utility_context)
            {
                return context_id;
            }
            changed.await;
        }
    })
    .await;

    found.map_err(|_| internal("the frame's utility world did not appear"))
}

fn context_of(context_id: Option<i64>) -> Result<String, Error> {
    context_id
        .map(context_name)
        .ok_or_else(|| internal("the page has no execution context yet"))
}

// A value in the utility script's wire format as plain JSON; None for undefined. Objects that
// have no JSON form (errors, dates, regular expressions, typed arrays) come back as their text.
pub(super) fn from_serialized(value: &Value) -> Option<Value> {
    let Some(object) = value.as_object() else {
        return Some(value.clone());
    };

    if let Some(kind) = object.get("v").and_then(Value::as_str) {
        return match kind {
            "null" => Some(Value::Null),
            "undefined" => None,
            // NaN and the infinities are not JSON.
            other => Some(json!(other)),
        };
    }
    if let Some(items) = object.get("a").and_then(Value::as_array) {
        return Some(Value::Array(
            items
                .iter()
                .map(|item| from_serialized(item).unwrap_or(Value::Null))
                .collect(),
        ));
    }
    if let Some(entries) = object.get("o").and_then(Value::as_array) {
        return Some(Value::Object(
            entries
                .iter()
                .filter_map(|entry| {
                    Some((
                        entry["k"].as_str()?.to_owned(),
                        from_serialized(&entry["v"])?,
                    ))
                })
                .collect(),
        ));
    }
    for key in ["d", "u", "bi"] {
        if let Some(text) = object.get(key) {
            return Some(text.clone());
        }
    }
    if let Some(error) = object.get("e") {
        return Some(error["s"].clone());
    }
    if let Some(regexp) = object.get("r") {
        let text = |key: &str| regexp[key].as_str().unwrap_or_default();

        return Some(json!(format!("/{}/{}", text("p"), text("f"))));
    }

    Some(Value::Null)
}

// The RemoteObject; EvaluationFailed when the code threw. Juggler writes undefined as
// `{ value: undefined }`, which reaches the wire as `{}`; it gets the type CDP gives it, or
// callers that tell undefined from a value see an object.
pub(super) fn remote_result(mut response: Value) -> Result<Value, Error> {
    if let Some(details) = response.get("exceptionDetails") {
        let message = details["text"]
            .as_str()
            .or_else(|| details["stack"].as_str())
            .unwrap_or("evaluation failed");

        return Err(Error::EvaluationFailed {
            message: message.chars().take(MAX_ERROR_CHARS).collect(),
        });
    }

    Ok(match response.get_mut("result") {
        Some(result) if result.as_object().is_some_and(|object| !object.is_empty()) => {
            result.take()
        }
        _ => json!({ "type": "undefined" }),
    })
}

impl PageSession {
    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value, Error> {
        self.juggler
            .call(Some(&self.juggler_session), method, params)
            .await
    }

    // A browser-level method aimed at this Tab's browser context.
    pub(crate) async fn browser_call(
        &self,
        method: &str,
        mut params: Value,
    ) -> Result<Value, Error> {
        if let Some(context_id) = &self.context_id {
            params["browserContextId"] = json!(context_id);
        }

        self.juggler.call(None, method, params).await
    }

    pub(crate) async fn evaluate(
        &self,
        context_id: Option<i64>,
        expression: &str,
        by_value: bool,
    ) -> Result<Value, Error> {
        let params = json!({
            "executionContextId": context_of(context_id)?,
            "expression": expression,
            "returnByValue": by_value,
        });

        remote_result(self.call("Runtime.evaluate", params).await?)
    }

    // Camoufox's `mw:` prefix sends a script to the page's own world. It rides on the shape of
    // Playwright's utility-script call, whose first argument it never reads, and answers in that
    // script's serialization.
    pub(crate) async fn evaluate_in_page(
        &self,
        context_id: Option<i64>,
        expression: &str,
    ) -> Result<Value, Error> {
        let params = json!({
            "executionContextId": context_of(context_id)?,
            "functionDeclaration": MAIN_WORLD_CALL,
            "returnByValue": true,
            "args": [
                { "value": null },
                { "value": false },
                { "value": true },
                { "value": format!("{MAIN_WORLD_PREFIX}{expression}") },
                { "value": 0 },
            ],
        });
        let result = remote_result(self.call("Runtime.callFunction", params).await?)?;

        Ok(match from_serialized(&result["value"]) {
            Some(value) => json!({ "value": value }),
            None => json!({ "type": "undefined" }),
        })
    }

    // Juggler has no `this` for a call: the object goes in as the first argument and a wrapper
    // applies it.
    pub(crate) async fn call_function(
        &self,
        context_id: Option<i64>,
        object_id: &str,
        declaration: &str,
        arguments: &[Value],
        by_value: bool,
    ) -> Result<Value, Error> {
        let mut args = vec![json!({ "objectId": object_id })];
        args.extend(arguments.iter().cloned());
        let params = json!({
            "executionContextId": context_of(context_id)?,
            "functionDeclaration": format!("function(self, ...args) {{ return ({declaration}).apply(self, args); }}"),
            "args": args,
            "returnByValue": by_value,
        });

        remote_result(self.call("Runtime.callFunction", params).await?)
    }

    pub(crate) fn release(&self, context_id: Option<i64>, object_id: &str) {
        let Some(context_id) = context_id else {
            return;
        };
        let session = self.clone();
        let object_id = object_id.to_owned();

        tokio::spawn(async move {
            let _ = session
                .call(
                    "Runtime.disposeObject",
                    json!({ "executionContextId": context_name(context_id), "objectId": object_id }),
                )
                .await;
        });
    }

    pub(crate) async fn content_quads(&self, element: &Element) -> Result<Vec<[f64; 8]>, Error> {
        let result = self
            .call(
                "Page.getContentQuads",
                json!({ "frameId": element.frame_id, "objectId": element.object_id }),
            )
            .await?;
        let point = |quad: &Value, name: &str| {
            [
                quad[name]["x"].as_f64().unwrap_or_default(),
                quad[name]["y"].as_f64().unwrap_or_default(),
            ]
        };

        Ok(result["quads"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|quad| {
                let [p1, p2, p3, p4] = ["p1", "p2", "p3", "p4"].map(|name| point(quad, name));

                [p1[0], p1[1], p2[0], p2[1], p3[0], p3[1], p4[0], p4[1]]
            })
            .collect())
    }

    pub(crate) async fn scroll_into_view(&self, element: &Element) -> Result<(), Error> {
        self.call(
            "Page.scrollIntoViewIfNeeded",
            json!({ "frameId": element.frame_id, "objectId": element.object_id }),
        )
        .await?;

        Ok(())
    }

    // The iframe element of `child`, as an object of the parent's utility world.
    pub(crate) async fn frame_element(
        &self,
        child: &str,
        parent: &str,
        state: &PageState,
    ) -> Result<Element, Error> {
        let context_id = state.frame(parent).and_then(|frame| frame.utility_context);
        let adopted = self
            .call(
                "Page.adoptNode",
                json!({ "frameId": child, "executionContextId": context_of(context_id)? }),
            )
            .await?;
        let object_id = adopted["remoteObject"]["objectId"]
            .as_str()
            .ok_or_else(|| gone("iframe element"))?;

        Ok(Element {
            frame_id: parent.to_owned(),
            session_id: self.session_id.clone(),
            object_id: object_id.to_owned(),
        })
    }

    pub(crate) async fn content_frame(
        &self,
        frame_id: &str,
        object_id: &str,
    ) -> Result<Option<String>, Error> {
        let node = self
            .call(
                "Page.describeNode",
                json!({ "frameId": frame_id, "objectId": object_id }),
            )
            .await?;

        Ok(node["contentFrameId"].as_str().map(str::to_owned))
    }
}
