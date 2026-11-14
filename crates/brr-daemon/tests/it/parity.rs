#![allow(clippy::unwrap_used)]

use crate::common;

use std::collections::HashSet;

use brr_core::{ErrorBody, InstanceId};
use brr_daemon::ops;
use serde_json::{Value, json};

use common::{listen, rpc, scratch_app};

const ROUTE_MISSING: &str = "route";
const EVENT_STREAM: &str = "text/event-stream";
// events.watch holds the request open for its whole timeout otherwise.
const WATCH_NOW: &str = "?timeout_ms=0";
const WATCH_OP: &str = "events.watch";
// Human overrides (LC6) belong to the person in the UI: no CLI command and no agent tool.
const NO_CLI: [&str; 3] = ["tab.take_over", "tab.hand_back", "tab.human_done"];
// The companion download is a zip a person installs into their browser.
const NO_MCP_TOOL: [&str; 4] = [
    "tab.take_over",
    "tab.hand_back",
    "tab.human_done",
    "extension.companion",
];

fn valid_name(name: &str) -> bool {
    let Some((group, action)) = name.split_once('.') else {
        return false;
    };
    let word =
        |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c == '_');

    word(group) && word(action)
}

fn unique<T: std::hash::Hash + Eq>(what: &str, values: impl Iterator<Item = T>) {
    let mut seen = HashSet::new();

    for value in values {
        assert!(seen.insert(value), "duplicate {what}");
    }
}

#[test]
fn names_follow_group_dot_action() {
    for op in ops::all() {
        let spec = op.spec;

        assert!(valid_name(spec.name), "{}", spec.name);
        assert!(
            spec.name.starts_with(&format!("{}.", spec.group)),
            "{} is not in group {}",
            spec.name,
            spec.group
        );
    }
}

#[test]
fn registry_entries_are_unique_and_complete() {
    let all = ops::all();

    unique("name", all.iter().map(|op| op.spec.name));
    unique(
        "route",
        all.iter()
            .map(|op| (op.spec.http.method, op.spec.http.path)),
    );
    unique("mcp tool", all.iter().filter_map(|op| op.spec.mcp_tool));
    unique(
        "cli path",
        all.iter()
            .map(|op| op.spec.cli)
            .filter(|cli| !cli.is_empty()),
    );

    for op in &all {
        assert_eq!(
            op.spec.mcp_tool.is_none(),
            NO_MCP_TOOL.contains(&op.spec.name),
            "{} mcp tool",
            op.spec.name
        );
        assert_eq!(
            op.spec.cli.is_empty(),
            NO_CLI.contains(&op.spec.name),
            "{} cli path",
            op.spec.name
        );
    }
}

#[tokio::test]
async fn every_tool_is_listed_with_its_positionals() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let body = rpc(addr, "", "tools/list", json!({})).await;
    let tools = body["result"]["tools"].as_array().unwrap();

    for op in ops::all() {
        let spec = op.spec;
        let Some(mcp_tool) = spec.mcp_tool else {
            continue;
        };
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == mcp_tool)
            .unwrap_or_else(|| panic!("{} is not listed", spec.name));

        for name in spec.cli_positional {
            assert!(
                tool["inputSchema"]["properties"].get(*name).is_some(),
                "{}: positional {name} is not an input property",
                spec.name
            );
        }
    }
}

#[tokio::test]
async fn every_route_is_mounted() {
    let (_dir, app) = scratch_app().await;
    let addr = listen(&app).await;
    let client = reqwest::Client::new();

    for op in ops::all() {
        let spec = op.spec;
        let mut path = spec.http.path.to_string();
        while let (Some(open), Some(close)) = (path.find('{'), path.find('}')) {
            path.replace_range(open..=close, &InstanceId::new().to_string());
        }
        if spec.name == WATCH_OP {
            path.push_str(WATCH_NOW);
        }

        let method = reqwest::Method::from_bytes(
            format!("{:?}", spec.http.method).to_uppercase().as_bytes(),
        )
        .unwrap();
        let response = client
            .request(method, format!("http://{addr}{path}"))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let streams = response
            .headers()
            .get("content-type")
            .is_some_and(|value| value == EVENT_STREAM);
        if streams {
            continue;
        }

        let body: Value = response.json().await.unwrap();
        if status.is_success() {
            continue;
        }

        let error: ErrorBody = serde_json::from_value(body).unwrap();
        let missing = error.code == "NotFound" && error.details["what"] == ROUTE_MISSING;
        assert!(!missing, "{} {path} is not mounted", spec.name);
    }
}
