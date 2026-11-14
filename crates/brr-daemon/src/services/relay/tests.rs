use serde_json::{Value, json};

use super::model::{Model, Out};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.6478.55 Safari/537.36";

fn model() -> Model {
    let mut model = Model::new(USER_AGENT.to_owned());
    model.on_extension(json!({
        "method": "chrome.tabs.onCreated",
        "params": [{ "id": 1, "url": "https://a.test/", "title": "A" }],
    }));

    model
}

fn cdp(outs: &[Out]) -> Vec<&Value> {
    outs.iter()
        .filter_map(|out| match out {
            Out::Cdp(frame) => Some(frame),
            Out::Extension(_) => None,
        })
        .collect()
}

fn extension(outs: &[Out]) -> Vec<&Value> {
    outs.iter()
        .filter_map(|out| match out {
            Out::Extension(command) => Some(command),
            Out::Cdp(_) => None,
        })
        .collect()
}

#[test]
fn browser_close_refused() {
    let mut model = model();

    let outs = model.on_cdp(json!({ "id": 7, "method": "Browser.close" }));

    assert!(extension(&outs).is_empty());
    let frames = cdp(&outs);
    assert_eq!(frames[0]["id"], 7);
    assert!(
        frames[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("never closes")
    );
}

#[test]
fn version_names_the_product() {
    let mut model = model();

    let outs = model.on_cdp(json!({ "id": 1, "method": "Browser.getVersion" }));

    assert_eq!(cdp(&outs)[0]["result"]["product"], "Chrome/126.0.6478.55");
    assert_eq!(cdp(&outs)[0]["result"]["protocolVersion"], "1.3");
}

#[test]
fn unknown_browser_method_is_refused() {
    let mut model = model();

    let outs = model.on_cdp(json!({ "id": 2, "method": "Storage.getCookies" }));

    assert!(extension(&outs).is_empty());
    assert!(
        cdp(&outs)[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not available over the companion extension")
    );
}

#[test]
fn discovery_lists_known_tabs_then_follows_changes() {
    let mut model = model();

    let outs = model.on_cdp(json!({
        "id": 1, "method": "Target.setDiscoverTargets", "params": { "discover": true },
    }));
    let frames = cdp(&outs);
    assert_eq!(frames[0]["id"], 1);
    assert_eq!(frames[1]["method"], "Target.targetCreated");
    assert_eq!(frames[1]["params"]["targetInfo"]["targetId"], "tab-1");

    let outs = model.on_extension(json!({
        "method": "chrome.tabs.onUpdated",
        "params": [1, { "title": "B" }, { "title": "B" }],
    }));
    assert_eq!(cdp(&outs)[0]["method"], "Target.targetInfoChanged");

    let outs = model.on_extension(json!({ "method": "chrome.tabs.onRemoved", "params": [1] }));
    assert_eq!(cdp(&outs)[0]["method"], "Target.targetDestroyed");
}

#[test]
fn attach_then_command_then_error_map_by_extension_id() {
    let mut model = model();

    let outs = model.on_cdp(json!({
        "id": 3, "method": "Target.attachToTarget", "params": { "targetId": "tab-1" },
    }));
    let attach = extension(&outs)[0].clone();
    assert_eq!(attach["method"], "chrome.debugger.attach");
    assert_eq!(attach["params"], json!([{ "tabId": 1 }, "1.3"]));

    let outs = model.on_extension(json!({ "id": attach["id"], "result": null }));
    let frames = cdp(&outs);
    assert_eq!(frames[0]["method"], "Target.attachedToTarget");
    assert_eq!(frames[1]["result"]["sessionId"], "brr-tab-1");

    let first = model.on_cdp(json!({
        "id": 4, "method": "Runtime.evaluate", "params": { "expression": "1+1" },
        "sessionId": "brr-tab-1",
    }));
    let second =
        model.on_cdp(json!({ "id": 5, "method": "Page.enable", "sessionId": "brr-tab-1" }));
    let (first, second) = (extension(&first)[0].clone(), extension(&second)[0].clone());
    assert_eq!(
        first["params"],
        json!([{ "tabId": 1 }, "Runtime.evaluate", { "expression": "1+1" }])
    );

    // Replies arrive out of order.
    let outs = model.on_extension(json!({ "id": second["id"], "error": "boom" }));
    let frame = cdp(&outs)[0].clone();
    assert_eq!(
        (frame["id"].clone(), frame["error"]["code"].clone()),
        (json!(5), json!(-32000))
    );
    assert_eq!(frame["error"]["message"], "boom");
    assert_eq!(frame["sessionId"], "brr-tab-1");

    let outs = model.on_extension(json!({ "id": first["id"], "result": { "value": 2 } }));
    assert_eq!(cdp(&outs)[0]["id"], 4);
    assert_eq!(cdp(&outs)[0]["result"]["value"], 2);
}

#[test]
fn child_sessions_route_with_their_own_id() {
    let mut model = model();
    let outs = model.on_cdp(json!({
        "id": 1, "method": "Target.attachToTarget", "params": { "targetId": "tab-1" },
    }));
    let id = extension(&outs)[0]["id"].clone();
    model.on_extension(json!({ "id": id, "result": null }));

    let outs = model.on_extension(json!({
        "method": "chrome.debugger.onEvent",
        "params": [{ "tabId": 1 }, "Target.attachedToTarget", { "sessionId": "child-9" }],
    }));
    assert_eq!(cdp(&outs)[0]["sessionId"], "brr-tab-1");

    let outs = model.on_cdp(json!({ "id": 2, "method": "Page.enable", "sessionId": "child-9" }));
    assert_eq!(
        extension(&outs)[0]["params"][0],
        json!({ "tabId": 1, "sessionId": "child-9" })
    );

    let outs = model.on_extension(json!({
        "method": "chrome.debugger.onEvent",
        "params": [{ "tabId": 1, "sessionId": "child-9" }, "Page.loadEventFired", {}],
    }));
    assert_eq!(cdp(&outs)[0]["sessionId"], "child-9");
}

#[test]
fn detach_event_ends_the_session_and_the_tab() {
    let mut model = model();
    model.on_cdp(json!({
        "id": 1, "method": "Target.setDiscoverTargets", "params": { "discover": true },
    }));
    let outs = model.on_cdp(json!({
        "id": 2, "method": "Target.attachToTarget", "params": { "targetId": "tab-1" },
    }));
    let id = extension(&outs)[0]["id"].clone();
    model.on_extension(json!({ "id": id, "result": null }));

    let outs = model.on_extension(json!({
        "method": "chrome.debugger.onDetach",
        "params": [{ "tabId": 1 }, "canceled_by_user"],
    }));
    let methods: Vec<_> = cdp(&outs)
        .iter()
        .map(|frame| frame["method"].clone())
        .collect();
    assert_eq!(
        methods,
        [
            json!("Target.detachedFromTarget"),
            json!("Target.targetDestroyed")
        ]
    );

    let outs = model.on_extension(json!({ "method": "chrome.tabs.onRemoved", "params": [1] }));
    assert!(outs.is_empty());
}
