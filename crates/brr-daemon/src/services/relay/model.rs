use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

const CDP_VERSION: &str = "1.3";
const TAB_PREFIX: &str = "tab-";
const SESSION_PREFIX: &str = "brr-tab-";
const CDP_ERROR: i64 = -32000;

pub enum Out {
    Cdp(Value),
    Extension(Value),
}

struct Tab {
    url: String,
    title: String,
    attached: bool,
}

enum Kind {
    Attach(u64),
    Detach(u64),
    Passthrough,
    Create,
    Fixed(Value),
}

// What a command sent to the extension is waiting for, so a reply (which carries only the
// extension's id) finds its CDP caller whatever order replies come in.
struct Reply {
    cdp_id: u64,
    session_id: Option<String>,
    kind: Kind,
}

// Browser-level CDP in front of chrome.debugger: one flat tab session per attached tab, child
// sessions learned from the tab's own Target.attachedToTarget events.
pub struct Model {
    user_agent: String,
    tabs: BTreeMap<u64, Tab>,
    discovering: bool,
    children: HashMap<String, u64>,
    pending: HashMap<u64, Reply>,
    next_id: u64,
}

fn tab_session(tab_id: u64) -> String {
    format!("{SESSION_PREFIX}{tab_id}")
}

fn target_id(tab_id: u64) -> String {
    format!("{TAB_PREFIX}{tab_id}")
}

fn parse_suffix(text: &str, prefix: &str) -> Option<u64> {
    text.strip_prefix(prefix)?.parse().ok()
}

fn ok(cdp_id: u64, session_id: Option<&str>, result: Value) -> Out {
    let mut frame = json!({ "id": cdp_id, "result": result });
    if let Some(session_id) = session_id {
        frame["sessionId"] = json!(session_id);
    }

    Out::Cdp(frame)
}

fn failed(cdp_id: u64, session_id: Option<&str>, message: &str) -> Out {
    let mut frame = json!({ "id": cdp_id, "error": { "code": CDP_ERROR, "message": message } });
    if let Some(session_id) = session_id {
        frame["sessionId"] = json!(session_id);
    }

    Out::Cdp(frame)
}

fn event(method: &str, params: Value, session_id: Option<&str>) -> Out {
    let mut frame = json!({ "method": method, "params": params });
    if let Some(session_id) = session_id {
        frame["sessionId"] = json!(session_id);
    }

    Out::Cdp(frame)
}

// "Mozilla/5.0 ... Chrome/126.0.0.0 Safari/537.36" -> "Chrome/126.0.0.0".
fn product(user_agent: &str) -> &str {
    user_agent
        .split_whitespace()
        .find(|token| token.starts_with("Chrome/") || token.starts_with("HeadlessChrome/"))
        .unwrap_or(user_agent)
}

impl Model {
    pub fn new(user_agent: String) -> Model {
        Model {
            user_agent,
            tabs: BTreeMap::new(),
            discovering: false,
            children: HashMap::new(),
            pending: HashMap::new(),
            next_id: 1,
        }
    }

    fn info(&self, tab_id: u64) -> Value {
        let tab = &self.tabs[&tab_id];

        json!({
            "targetId": target_id(tab_id),
            "type": "page",
            "title": tab.title,
            "url": tab.url,
            "attached": tab.attached,
            "canAccessOpener": false,
        })
    }

    fn send(&mut self, reply: Reply, method: &str, params: Value) -> Vec<Out> {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, reply);

        vec![Out::Extension(
            json!({ "id": id, "method": method, "params": params }),
        )]
    }

    // Frames come in by value: screenshots and screencast frames are megabytes of base64 that
    // must move through, not be copied.
    pub fn on_cdp(&mut self, mut frame: Value) -> Vec<Out> {
        let Some(cdp_id) = frame["id"].as_u64() else {
            return Vec::new();
        };
        let mut params = frame["params"].take();
        if !params.is_object() {
            params = json!({});
        }
        let method = frame["method"].as_str().unwrap_or_default();

        match frame["sessionId"].as_str() {
            Some(session_id) => self.session_command(cdp_id, session_id, method, params),
            None => self.browser_command(cdp_id, method, &params),
        }
    }

    fn session_command(
        &mut self,
        cdp_id: u64,
        session_id: &str,
        method: &str,
        params: Value,
    ) -> Vec<Out> {
        let source = if let Some(tab_id) = parse_suffix(session_id, SESSION_PREFIX) {
            self.tabs
                .get(&tab_id)
                .is_some_and(|tab| tab.attached)
                .then(|| json!({ "tabId": tab_id }))
        } else {
            self.children
                .get(session_id)
                .map(|tab_id| json!({ "tabId": tab_id, "sessionId": session_id }))
        };
        let Some(source) = source else {
            return vec![failed(cdp_id, Some(session_id), "no such session")];
        };

        let reply = Reply {
            cdp_id,
            session_id: Some(session_id.to_owned()),
            kind: Kind::Passthrough,
        };

        self.send(
            reply,
            "chrome.debugger.sendCommand",
            json!([source, method, params]),
        )
    }

    fn browser_command(&mut self, cdp_id: u64, method: &str, params: &Value) -> Vec<Out> {
        let tab_id = params["targetId"]
            .as_str()
            .and_then(|target| parse_suffix(target, TAB_PREFIX))
            .filter(|tab_id| self.tabs.contains_key(tab_id));
        let reply = |kind| Reply {
            cdp_id,
            session_id: None,
            kind,
        };

        match method {
            "Browser.getVersion" => vec![ok(
                cdp_id,
                None,
                json!({
                    "protocolVersion": CDP_VERSION,
                    "product": product(&self.user_agent),
                    "userAgent": self.user_agent,
                }),
            )],
            "Browser.close" => vec![failed(cdp_id, None, "brr never closes a Live browser")],
            "Target.setDiscoverTargets" => {
                self.discovering = params["discover"].as_bool().unwrap_or(false);
                let mut outs = vec![ok(cdp_id, None, json!({}))];
                if self.discovering {
                    for tab_id in self.tabs.keys() {
                        let params = json!({ "targetInfo": self.info(*tab_id) });
                        outs.push(event("Target.targetCreated", params, None));
                    }
                }

                outs
            }
            "Target.attachToTarget" => {
                let Some(tab_id) = tab_id else {
                    return vec![failed(cdp_id, None, "no such target")];
                };
                if self.tabs[&tab_id].attached {
                    return vec![ok(
                        cdp_id,
                        None,
                        json!({ "sessionId": tab_session(tab_id) }),
                    )];
                }

                self.send(
                    reply(Kind::Attach(tab_id)),
                    "chrome.debugger.attach",
                    json!([{ "tabId": tab_id }, CDP_VERSION]),
                )
            }
            "Target.detachFromTarget" => {
                let tab_id = params["sessionId"]
                    .as_str()
                    .and_then(|session| parse_suffix(session, SESSION_PREFIX))
                    .filter(|tab_id| self.tabs.get(tab_id).is_some_and(|tab| tab.attached));
                let Some(tab_id) = tab_id else {
                    return vec![failed(cdp_id, None, "no such session")];
                };

                self.send(
                    reply(Kind::Detach(tab_id)),
                    "chrome.debugger.detach",
                    json!([{ "tabId": tab_id }]),
                )
            }
            "Target.createTarget" => {
                let url = params["url"].as_str().unwrap_or("about:blank");

                self.send(
                    reply(Kind::Create),
                    "chrome.tabs.create",
                    json!([{ "url": url, "active": false }]),
                )
            }
            "Target.closeTarget" => {
                let Some(tab_id) = tab_id else {
                    return vec![failed(cdp_id, None, "no such target")];
                };

                self.send(
                    reply(Kind::Fixed(json!({ "success": true }))),
                    "chrome.tabs.remove",
                    json!([tab_id]),
                )
            }
            "Target.activateTarget" => {
                let Some(tab_id) = tab_id else {
                    return vec![failed(cdp_id, None, "no such target")];
                };

                self.send(
                    reply(Kind::Fixed(json!({}))),
                    "chrome.tabs.update",
                    json!([tab_id, { "active": true }]),
                )
            }
            _ => vec![failed(
                cdp_id,
                None,
                &format!("{method} is not available over the companion extension"),
            )],
        }
    }

    pub fn on_extension(&mut self, mut message: Value) -> Vec<Out> {
        match message["id"].as_u64() {
            Some(id) => self.on_reply(id, message),
            None => {
                let params = message["params"].take();

                self.on_event(message["method"].as_str().unwrap_or_default(), params)
            }
        }
    }

    fn on_reply(&mut self, id: u64, mut message: Value) -> Vec<Out> {
        let Some(Reply {
            cdp_id,
            session_id,
            kind,
        }) = self.pending.remove(&id)
        else {
            return Vec::new();
        };
        let session = session_id.as_deref();
        if let Some(error) = message["error"].as_str() {
            return vec![failed(cdp_id, session, error)];
        }
        let result = message["result"].take();

        match kind {
            Kind::Attach(tab_id) => {
                let Some(tab) = self.tabs.get_mut(&tab_id) else {
                    return vec![failed(cdp_id, None, "the tab closed while attaching")];
                };
                tab.attached = true;
                let session_id = tab_session(tab_id);
                let params = json!({
                    "sessionId": session_id,
                    "targetInfo": self.info(tab_id),
                    "waitingForDebugger": false,
                });

                vec![
                    event("Target.attachedToTarget", params, None),
                    ok(cdp_id, None, json!({ "sessionId": session_id })),
                ]
            }
            Kind::Detach(tab_id) => {
                let mut outs = self.detached(tab_id);
                outs.push(ok(cdp_id, None, json!({})));

                outs
            }
            Kind::Create => match result["id"].as_u64() {
                Some(tab_id) => vec![ok(cdp_id, None, json!({ "targetId": target_id(tab_id) }))],
                None => vec![failed(cdp_id, None, "the extension created no tab")],
            },
            Kind::Fixed(result) => vec![ok(cdp_id, None, result)],
            Kind::Passthrough => vec![ok(cdp_id, session, result)],
        }
    }

    // The session ends; the tab may live on.
    fn detached(&mut self, tab_id: u64) -> Vec<Out> {
        let Some(tab) = self.tabs.get_mut(&tab_id) else {
            return Vec::new();
        };
        if !std::mem::take(&mut tab.attached) {
            return Vec::new();
        }
        self.children.retain(|_, parent| *parent != tab_id);
        let params = json!({ "sessionId": tab_session(tab_id), "targetId": target_id(tab_id) });

        vec![event("Target.detachedFromTarget", params, None)]
    }

    fn remove(&mut self, tab_id: u64) -> Vec<Out> {
        let mut outs = self.detached(tab_id);
        if self.tabs.remove(&tab_id).is_some() && self.discovering {
            let params = json!({ "targetId": target_id(tab_id) });
            outs.push(event("Target.targetDestroyed", params, None));
        }

        outs
    }

    // A created or updated tab: the same record either way.
    fn upsert(&mut self, tab: &Value) -> Vec<Out> {
        let Some(tab_id) = tab["id"].as_u64() else {
            return Vec::new();
        };
        let text = |key: &str| tab[key].as_str().map(str::to_owned);
        let known = self.tabs.contains_key(&tab_id);
        let entry = self.tabs.entry(tab_id).or_insert_with(|| Tab {
            url: String::new(),
            title: String::new(),
            attached: false,
        });
        if let Some(url) = text("url").or_else(|| text("pendingUrl")) {
            entry.url = url;
        }
        if let Some(title) = text("title") {
            entry.title = title;
        }
        if !self.discovering {
            return Vec::new();
        }

        let method = if known {
            "Target.targetInfoChanged"
        } else {
            "Target.targetCreated"
        };

        vec![event(
            method,
            json!({ "targetInfo": self.info(tab_id) }),
            None,
        )]
    }

    fn on_event(&mut self, method: &str, mut params: Value) -> Vec<Out> {
        match method {
            "chrome.tabs.onCreated" => self.upsert(&params[0]),
            "chrome.tabs.onUpdated" => {
                let mut tab = params[2].take();
                tab["id"] = params[0].take();

                self.upsert(&tab)
            }
            "chrome.tabs.onRemoved" => params[0]
                .as_u64()
                .map_or_else(Vec::new, |id| self.remove(id)),
            "chrome.debugger.onDetach" => params[0]["tabId"]
                .as_u64()
                .map_or_else(Vec::new, |id| self.remove(id)),
            "chrome.debugger.onEvent" => {
                let payload = params[2].take();

                self.debugger_event(&params[0], &params[1], payload)
            }
            _ => Vec::new(),
        }
    }

    fn debugger_event(&mut self, source: &Value, method: &Value, params: Value) -> Vec<Out> {
        let (Some(tab_id), Some(method)) = (source["tabId"].as_u64(), method.as_str()) else {
            return Vec::new();
        };
        if !self.tabs.get(&tab_id).is_some_and(|tab| tab.attached) {
            return Vec::new();
        }

        match (method, params["sessionId"].as_str()) {
            ("Target.attachedToTarget", Some(child)) => {
                self.children.insert(child.to_owned(), tab_id);
            }
            ("Target.detachedFromTarget", Some(child)) => {
                self.children.remove(child);
            }
            _ => {}
        }
        let session_id = source["sessionId"]
            .as_str()
            .map_or_else(|| tab_session(tab_id), str::to_owned);

        vec![event(method, params, Some(&session_id))]
    }
}
