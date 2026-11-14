use std::time::Duration;

use brr_cdp::{Cdp, SessionId};
use brr_core::{Error, WaitUntil};
use serde_json::{Value, json};

use super::actions::Point;
use super::{ELEMENT_GROUP, Element, MAX_ERROR_CHARS, PageSession, PageState, Session};
use crate::camoufox;

const IFRAME_INSET: &str = "function() { const style = getComputedStyle(this); return { x: this.clientLeft + parseFloat(style.paddingLeft), y: this.clientTop + parseFloat(style.paddingTop) }; }";

// Engine-specific primitives. Everything above these is shared between Chromium and Camoufox.
impl Session {
    pub(super) fn chromium(&self) -> Result<&PageSession, Error> {
        match self {
            Session::Chromium(session) => Ok(session),
            Session::Camoufox(_) => Err(camoufox::unsupported()),
        }
    }

    pub(super) fn cdp(&self) -> Result<&Cdp, Error> {
        Ok(&self.chromium()?.cdp)
    }

    pub(super) fn browser_name(&self) -> &'static str {
        match self {
            Session::Chromium(_) => "chromium",
            Session::Camoufox(_) => "firefox",
        }
    }

    pub(super) fn main_session(&self) -> &SessionId {
        match self {
            Session::Chromium(session) => &session.session_id,
            Session::Camoufox(session) => &session.session_id,
        }
    }

    // The RemoteObject; EvaluationFailed when the expression threw.
    pub(super) async fn evaluate(
        &self,
        session_id: &SessionId,
        context_id: Option<i64>,
        expression: &str,
        by_value: bool,
        timeout: Option<Duration>,
    ) -> Result<Value, Error> {
        match self {
            Session::Chromium(session) => {
                let mut params = json!({
                    "expression": expression,
                    "returnByValue": by_value,
                    "awaitPromise": true,
                });
                if let Some(context_id) = context_id {
                    params["contextId"] = json!(context_id);
                }
                if let Some(timeout) = timeout {
                    params["timeout"] = json!(crate::chromium::millis(timeout));
                }

                // An awaited promise can outlast the default call limit; the caller's timeout rules.
                let answer = match timeout {
                    Some(timeout) => {
                        session
                            .cdp
                            .call_within(Some(session_id), "Runtime.evaluate", params, timeout)
                            .await?
                    }
                    None => {
                        session
                            .cdp
                            .call(Some(session_id), "Runtime.evaluate", params)
                            .await?
                    }
                };

                runtime_result(answer)
            }
            Session::Camoufox(session) => session.evaluate(context_id, expression, by_value).await,
        }
    }

    // Runs in the page's own world, where its scripts' globals are visible. Camoufox keeps its
    // default context isolated from the page and offers the page's world on request.
    pub(super) async fn evaluate_in_page(
        &self,
        session_id: &SessionId,
        context_id: Option<i64>,
        expression: &str,
        timeout: Duration,
    ) -> Result<Value, Error> {
        match self {
            Session::Chromium(_) => {
                self.evaluate(session_id, context_id, expression, true, Some(timeout))
                    .await
            }
            Session::Camoufox(session) => session.evaluate_in_page(context_id, expression).await,
        }
    }

    // `declaration` runs with the object `object_id` as `this`; `arguments` are call arguments
    // by value or by object id.
    pub(super) async fn call_function(
        &self,
        session_id: &SessionId,
        context_id: Option<i64>,
        object_id: &str,
        declaration: &str,
        arguments: &[Value],
        by_value: bool,
    ) -> Result<Value, Error> {
        match self {
            Session::Chromium(session) => {
                let params = json!({
                    "objectId": object_id,
                    "functionDeclaration": declaration,
                    "arguments": arguments,
                    "returnByValue": by_value,
                    "awaitPromise": true,
                    "objectGroup": ELEMENT_GROUP,
                });

                runtime_result(
                    session
                        .cdp
                        .call(Some(session_id), "Runtime.callFunctionOn", params)
                        .await?,
                )
            }
            Session::Camoufox(session) => {
                session
                    .call_function(context_id, object_id, declaration, arguments, by_value)
                    .await
            }
        }
    }

    // The utility world of a frame's current document. Chromium creates it on demand;
    // Camoufox's exists from the Tab's init script, so this waits for its context event.
    pub(super) async fn utility_world(
        &self,
        session_id: &SessionId,
        frame_id: &str,
        state: &PageState,
    ) -> Result<i64, Error> {
        match self {
            Session::Chromium(session) => {
                super::create_world(&session.cdp, session_id, frame_id, state).await
            }
            Session::Camoufox(_) => camoufox::utility_context(state, frame_id).await,
        }
    }

    // Fire and forget: a page stalled on a dialog would hold the caller until the call times out.
    pub(super) fn release(&self, element: &Element, context_id: Option<i64>) {
        match self {
            Session::Chromium(session) => {
                let cdp = session.cdp.clone();
                let session_id = element.session_id.clone();
                let object_id = element.object_id.clone();

                tokio::spawn(async move {
                    let _ = cdp
                        .call(
                            Some(&session_id),
                            "Runtime.releaseObject",
                            json!({ "objectId": object_id }),
                        )
                        .await;
                });
            }
            Session::Camoufox(session) => session.release(context_id, &element.object_id),
        }
    }

    pub(super) async fn content_quads(&self, element: &Element) -> Result<Vec<[f64; 8]>, Error> {
        match self {
            Session::Chromium(session) => {
                let result = session
                    .cdp
                    .call(
                        Some(&element.session_id),
                        "DOM.getContentQuads",
                        json!({ "objectId": element.object_id }),
                    )
                    .await?;

                Ok(result["quads"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|quad| {
                        let numbers: Vec<f64> =
                            quad.as_array()?.iter().filter_map(Value::as_f64).collect();

                        <[f64; 8]>::try_from(numbers).ok()
                    })
                    .collect())
            }
            Session::Camoufox(session) => session.content_quads(element).await,
        }
    }

    pub(super) async fn scroll_into_view(&self, element: &Element) -> Result<(), Error> {
        match self {
            Session::Chromium(session) => {
                session
                    .cdp
                    .call(
                        Some(&element.session_id),
                        "DOM.scrollIntoViewIfNeeded",
                        json!({ "objectId": element.object_id }),
                    )
                    .await?;

                Ok(())
            }
            Session::Camoufox(session) => session.scroll_into_view(element).await,
        }
    }

    // Where the content of `child` starts in its parent's local root.
    pub(super) async fn frame_origin(
        &self,
        child: &str,
        parent: &str,
        state: &PageState,
    ) -> Result<Point, Error> {
        match self {
            Session::Chromium(session) => {
                let parent_session = state
                    .frame(parent)
                    .map(|frame| frame.session_id)
                    .ok_or_else(|| crate::common::gone("frame"))?;
                let owner = session
                    .cdp
                    .call(
                        Some(&parent_session),
                        "DOM.getFrameOwner",
                        json!({ "frameId": child }),
                    )
                    .await?;
                let model = session
                    .cdp
                    .call(
                        Some(&parent_session),
                        "DOM.getBoxModel",
                        json!({ "backendNodeId": owner["backendNodeId"] }),
                    )
                    .await?;
                // The content box starts after the iframe's border and padding.
                let content = &model["model"]["content"];

                Ok(Point {
                    x: content[0].as_f64().unwrap_or_default(),
                    y: content[1].as_f64().unwrap_or_default(),
                })
            }
            Session::Camoufox(session) => {
                let owner = session.frame_element(child, parent, state).await?;
                let context_id = state.frame(parent).and_then(|frame| frame.utility_context);
                // Juggler's quads are the border box; the content starts after border and padding.
                let quads = session.content_quads(&owner).await;
                let inset = session
                    .call_function(context_id, &owner.object_id, IFRAME_INSET, &[], true)
                    .await;
                self.release(&owner, context_id);

                let quad = quads?
                    .into_iter()
                    .next()
                    .ok_or_else(|| crate::common::gone("iframe"))?;
                let inset = &inset?["value"];

                Ok(Point {
                    x: quad[0] + inset["x"].as_f64().unwrap_or_default(),
                    y: quad[1] + inset["y"].as_f64().unwrap_or_default(),
                })
            }
        }
    }

    // The iframe element that holds `child`, resolved in the parent's utility world.
    pub(super) async fn frame_element(
        &self,
        child: &str,
        parent: &str,
        state: &PageState,
        session_id: &SessionId,
        context_id: Option<i64>,
    ) -> Result<Element, Error> {
        match self {
            Session::Chromium(session) => {
                let owner = session
                    .cdp
                    .call(
                        Some(session_id),
                        "DOM.getFrameOwner",
                        json!({ "frameId": child }),
                    )
                    .await?;
                let resolved = session
                    .cdp
                    .call(
                        Some(session_id),
                        "DOM.resolveNode",
                        json!({
                            "backendNodeId": owner["backendNodeId"],
                            "executionContextId": context_id,
                            "objectGroup": ELEMENT_GROUP,
                        }),
                    )
                    .await?;
                let object_id =
                    resolved["object"]["objectId"]
                        .as_str()
                        .ok_or_else(|| Error::Internal {
                            message: "an iframe element did not resolve".to_owned(),
                        })?;

                Ok(Element {
                    frame_id: parent.to_owned(),
                    session_id: session_id.clone(),
                    object_id: object_id.to_owned(),
                })
            }
            Session::Camoufox(session) => session.frame_element(child, parent, state).await,
        }
    }

    // The frame an iframe element holds, if it is one of this Tab's.
    pub(super) async fn content_frame(
        &self,
        session_id: &SessionId,
        frame_id: &str,
        object_id: &str,
    ) -> Result<Option<String>, Error> {
        match self {
            Session::Chromium(session) => {
                let node = session
                    .cdp
                    .call(
                        Some(session_id),
                        "DOM.describeNode",
                        json!({ "objectId": object_id }),
                    )
                    .await?;

                Ok(node["node"]["frameId"].as_str().map(str::to_owned))
            }
            Session::Camoufox(session) => session.content_frame(frame_id, object_id).await,
        }
    }

    // Closes the references a read-only call left in the page.
    pub(super) fn drop_object(
        &self,
        session_id: &SessionId,
        context_id: Option<i64>,
        object_id: &str,
    ) {
        let element = Element {
            frame_id: String::new(),
            session_id: session_id.clone(),
            object_id: object_id.to_owned(),
        };
        self.release(&element, context_id);
    }

    pub(crate) async fn dispatch_mouse(&self, params: Value) -> Result<(), Error> {
        match self {
            Session::Chromium(session) => session.dispatch_mouse(params).await,
            Session::Camoufox(session) => session.dispatch_mouse(&params).await,
        }
    }

    pub(crate) async fn dispatch_key(&self, params: Value) -> Result<(), Error> {
        match self {
            Session::Chromium(session) => session.dispatch_key(params).await,
            Session::Camoufox(session) => session.dispatch_key(&params).await,
        }
    }

    pub(crate) async fn insert_text(&self, text: &str) -> Result<(), Error> {
        match self {
            Session::Chromium(session) => session.insert_text(text).await,
            Session::Camoufox(session) => session.insert_text(text).await,
        }
    }

    // `session_id` is the session the dialog opened on.
    pub(crate) async fn handle_dialog(
        &self,
        session_id: &SessionId,
        accept: bool,
        prompt_text: Option<&str>,
    ) -> Result<(), Error> {
        match self {
            Session::Chromium(session) => {
                session.handle_dialog(session_id, accept, prompt_text).await
            }
            Session::Camoufox(session) => session.handle_dialog(accept, prompt_text).await,
        }
    }

    // Ok(false) when there is no entry that way.
    pub(crate) async fn go_history(
        &self,
        delta: i64,
        main_frame: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<bool, Error> {
        match self {
            Session::Chromium(session) => {
                session
                    .go_history(delta, main_frame, wait_until, timeout)
                    .await
            }
            Session::Camoufox(session) => {
                session
                    .go_history(delta, main_frame, wait_until, timeout)
                    .await
            }
        }
    }

    pub(crate) async fn reload(
        &self,
        main_frame: &str,
        wait_until: WaitUntil,
        timeout: Duration,
    ) -> Result<(), Error> {
        match self {
            Session::Chromium(session) => session.reload(main_frame, wait_until, timeout).await,
            Session::Camoufox(session) => session.reload(main_frame, wait_until, timeout).await,
        }
    }

    pub(crate) async fn grant_geolocation(&self) -> Result<(), Error> {
        self.chromium()?.grant_geolocation().await
    }
}

fn runtime_result(mut response: Value) -> Result<Value, Error> {
    if let Some(details) = response.get("exceptionDetails") {
        let message = details["exception"]["description"]
            .as_str()
            .or_else(|| details["text"].as_str())
            .unwrap_or("evaluation failed");

        return Err(Error::EvaluationFailed {
            message: message.chars().take(MAX_ERROR_CHARS).collect(),
        });
    }

    Ok(response["result"].take())
}
