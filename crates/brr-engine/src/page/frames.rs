use std::collections::HashMap;

use brr_cdp::SessionId;
use serde::Deserialize;
use serde_json::Value;

use super::UTILITY_WORLD;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub parent: Option<String>,
    pub url: String,
    pub session_id: SessionId,
    pub frame_seq: u32,
    pub utility_context: Option<i64>,
    pub main_context: Option<i64>,
    // Remote object id of the injected script instance living in the utility context.
    pub injected: Option<String>,
}

// Every frame of a page, across the page session and its out-of-process iframe sessions.
#[derive(Debug, Default)]
pub struct FrameTree {
    pub frames: HashMap<String, Frame>,
    pub main: String,
    pub next_seq: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameAttached {
    frame_id: String,
    parent_frame_id: Option<String>,
}

#[derive(Deserialize)]
struct FrameNavigated {
    frame: FramePayload,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FramePayload {
    id: String,
    parent_id: Option<String>,
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameDetached {
    frame_id: String,
    // Juggler sends none; Chromium sends "swap" for a frame that moved to another process.
    #[serde(default)]
    reason: String,
}

#[derive(Deserialize)]
struct NavigationCommitted {
    #[serde(rename = "frameId")]
    frame_id: String,
    url: String,
}

#[derive(Deserialize)]
struct ContextCreated {
    context: ContextPayload,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextPayload {
    id: i64,
    name: String,
    aux_data: Option<AuxData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuxData {
    frame_id: Option<String>,
    is_default: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextDestroyed {
    execution_context_id: i64,
}

impl Frame {
    fn clear_contexts(&mut self) {
        self.utility_context = None;
        self.main_context = None;
        self.injected = None;
    }
}

impl FrameTree {
    fn insert(&mut self, session_id: &SessionId, frame_id: &str, parent: Option<String>) {
        let frame_seq = self.next_seq;
        self.next_seq += 1;
        self.frames.insert(
            frame_id.to_owned(),
            Frame {
                parent,
                url: String::new(),
                session_id: session_id.clone(),
                frame_seq,
                utility_context: None,
                main_context: None,
                injected: None,
            },
        );
    }

    fn ensure(&mut self, session_id: &SessionId, frame_id: &str, parent: Option<String>) {
        match self.frames.get_mut(frame_id) {
            Some(frame) => {
                if parent.is_some() {
                    frame.parent = parent;
                }
            }
            None => self.insert(session_id, frame_id, parent),
        }
    }

    // Frames seen first from a frame tree (attach, or a getFrameTree answer).
    pub fn seed(&mut self, session_id: &SessionId, tree: &Value) -> Vec<String> {
        let mut seeded = Vec::new();
        self.seed_node(session_id, tree, None, &mut seeded);

        seeded
    }

    fn seed_node(
        &mut self,
        session_id: &SessionId,
        node: &Value,
        parent: Option<&str>,
        seeded: &mut Vec<String>,
    ) {
        let Some(id) = node["frame"]["id"].as_str() else {
            return;
        };
        let parent = node["frame"]["parentId"]
            .as_str()
            .or(parent)
            .map(str::to_owned);
        if parent.is_none() && self.main.is_empty() {
            id.clone_into(&mut self.main);
        }
        self.ensure(session_id, id, parent);
        if let Some(frame) = self.frames.get_mut(id) {
            node["frame"]["url"]
                .as_str()
                .unwrap_or_default()
                .clone_into(&mut frame.url);
        }
        seeded.push(id.to_owned());

        if let Some(children) = node["childFrames"].as_array() {
            for child in children {
                self.seed_node(session_id, child, Some(id), seeded);
            }
        }
    }

    // A new document: contexts and the injected instance die with the old one, and the new
    // sequence number makes refs of the old document fail.
    pub fn navigated(&mut self, session_id: &SessionId, frame_id: &str, parent: Option<String>) {
        let Some(frame) = self.frames.get_mut(frame_id) else {
            self.insert(session_id, frame_id, parent);

            return;
        };
        let frame_seq = self.next_seq;
        self.next_seq += 1;

        frame.clear_contexts();
        frame.frame_seq = frame_seq;
        frame.session_id = session_id.clone();
        if parent.is_some() {
            frame.parent = parent;
        }
    }

    // Firefox creates a document's contexts before it reports the commit, so they are kept;
    // the context events replace the old ones.
    pub fn committed(&mut self, session_id: &SessionId, frame_id: &str, url: String) {
        if !self.frames.contains_key(frame_id) {
            self.insert(session_id, frame_id, None);
        }
        let frame_seq = self.next_seq;
        self.next_seq += 1;
        if let Some(frame) = self.frames.get_mut(frame_id) {
            frame.frame_seq = frame_seq;
            frame.url = url;
        }
    }

    // `swap` means the frame moved to another process and lives on in a child session.
    pub fn detached(&mut self, frame_id: &str, swap: bool) {
        if swap {
            if let Some(frame) = self.frames.get_mut(frame_id) {
                frame.clear_contexts();
            }

            return;
        }

        self.remove_where(|id, _| id == frame_id);
    }

    fn remove_where(&mut self, doomed: impl Fn(&str, &Frame) -> bool) {
        let mut removed: Vec<String> = self
            .frames
            .iter()
            .filter(|(id, frame)| doomed(id, frame))
            .map(|(id, _)| id.clone())
            .collect();

        while !removed.is_empty() {
            for id in &removed {
                self.frames.remove(id);
            }
            removed = self
                .frames
                .iter()
                .filter(|(_, frame)| {
                    frame
                        .parent
                        .as_ref()
                        .is_some_and(|parent| removed.contains(parent))
                })
                .map(|(id, _)| id.clone())
                .collect();
        }
    }

    // The topmost frame in the same process: its viewport is the coordinate space of quads.
    pub fn local_root(&self, frame_id: &str) -> String {
        let mut current = frame_id;

        while let Some(frame) = self.frames.get(current)
            && let Some(parent) = &frame.parent
            && self
                .frames
                .get(parent)
                .is_some_and(|parent_frame| parent_frame.session_id == frame.session_id)
        {
            current = parent;
        }

        current.to_owned()
    }

    pub fn session_detached(&mut self, session_id: &SessionId) {
        self.remove_where(|_, frame| frame.session_id == *session_id);
    }

    // The frame is an out-of-process iframe from now on: the child session owns it. A frame the
    // parent session has not announced yet is inserted under its parent, if this tree has it.
    pub fn adopt(&mut self, frame_id: &str, session_id: &SessionId, parent: Option<&str>) -> bool {
        if let Some(frame) = self.frames.get_mut(frame_id) {
            frame.session_id = session_id.clone();
            frame.clear_contexts();

            return true;
        }
        match parent {
            Some(parent) if self.frames.contains_key(parent) => {
                self.insert(session_id, frame_id, Some(parent.to_owned()));

                true
            }
            _ => false,
        }
    }

    pub fn context_created(
        &mut self,
        session_id: &SessionId,
        frame_id: &str,
        name: &str,
        is_default: bool,
        context_id: i64,
    ) {
        self.ensure(session_id, frame_id, None);
        let Some(frame) = self.frames.get_mut(frame_id) else {
            return;
        };
        if frame.session_id != *session_id {
            return;
        }

        if name != UTILITY_WORLD {
            if is_default {
                frame.main_context = Some(context_id);
            }

            return;
        }
        self.set_utility(frame_id, context_id);
    }

    // A different context means the injected instance of the old one is gone.
    pub fn set_utility(&mut self, frame_id: &str, context_id: i64) {
        let Some(frame) = self.frames.get_mut(frame_id) else {
            return;
        };
        if frame.utility_context != Some(context_id) {
            frame.utility_context = Some(context_id);
            frame.injected = None;
        }
    }

    pub fn context_destroyed(&mut self, session_id: &SessionId, context_id: i64) {
        for frame in self.frames.values_mut() {
            if frame.session_id != *session_id {
                continue;
            }
            if frame.utility_context == Some(context_id) {
                frame.utility_context = None;
                frame.injected = None;
            }
            if frame.main_context == Some(context_id) {
                frame.main_context = None;
            }
        }
    }

    pub fn contexts_cleared(&mut self, session_id: &SessionId) {
        for frame in self.frames.values_mut() {
            if frame.session_id == *session_id {
                frame.clear_contexts();
            }
        }
    }

    // Ignores a method this tree does not track, or params of an unexpected shape.
    pub fn apply(&mut self, session_id: &SessionId, method: &str, params: &Value) {
        match method {
            "Page.frameAttached" => {
                if let Ok(event) = FrameAttached::deserialize(params) {
                    self.ensure(session_id, &event.frame_id, event.parent_frame_id);
                }
            }
            "Page.frameNavigated" => {
                if let Ok(event) = FrameNavigated::deserialize(params) {
                    self.navigated(session_id, &event.frame.id, event.frame.parent_id);
                    if let Some(frame) = self.frames.get_mut(&event.frame.id) {
                        frame.url = event.frame.url;
                    }
                }
            }
            "Page.navigationCommitted" => {
                if let Ok(event) = NavigationCommitted::deserialize(params) {
                    self.committed(session_id, &event.frame_id, event.url);
                }
            }
            "Page.frameDetached" => {
                if let Ok(event) = FrameDetached::deserialize(params) {
                    self.detached(&event.frame_id, event.reason == "swap");
                }
            }
            "Runtime.executionContextCreated" => {
                if let Ok(event) = ContextCreated::deserialize(params)
                    && let Some(AuxData {
                        frame_id: Some(frame_id),
                        is_default,
                    }) = event.context.aux_data
                {
                    self.context_created(
                        session_id,
                        &frame_id,
                        &event.context.name,
                        is_default.unwrap_or(false),
                        event.context.id,
                    );
                }
            }
            "Runtime.executionContextDestroyed" => {
                if let Ok(event) = ContextDestroyed::deserialize(params) {
                    self.context_destroyed(session_id, event.execution_context_id);
                }
            }
            "Runtime.executionContextsCleared" => self.contexts_cleared(session_id),
            _ => {}
        }
    }
}
