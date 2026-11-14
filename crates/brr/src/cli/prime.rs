use std::fmt::Write;

use brr_daemon::{Boot, remote::Remote};
use serde_json::{Value, json};

use super::format;

pub const NAME: &str = "prime";

const GUIDE: &str = include_str!("prime.md");
const MAX_LISTED: u32 = 20;
const LIVE_STATES: &str = "queued,starting,running,idle";
const BUILD_LIST: &str = "build.list";
const PROFILE_LIST: &str = "profile.list";
const INSTANCE_LIST: &str = "instance.list";

// Never starts the daemon: hooks run this on every session start.
pub async fn run(boot: &Boot) -> anyhow::Result<()> {
    format::data(GUIDE)?;

    let mut out = String::from("## Right now\n\n");
    let bind = boot.settings.server.bind;
    writeln!(
        out,
        "- Web UI: http://{bind}/ · MCP over HTTP: http://{bind}/mcp"
    )?;

    let remote = Remote::new(&boot.paths.socket(&boot.settings))?;
    if !remote.is_up().await {
        writeln!(
            out,
            "- The daemon is not running. The first `brr` command or MCP call starts it."
        )?;

        return Ok(format::data(&out)?);
    }

    let limit = json!({ "limit": MAX_LISTED });
    let builds = remote.call(BUILD_LIST, &limit, None).await?;
    let profiles = remote.call(PROFILE_LIST, &limit, None).await?;
    let instances = remote
        .call(
            INSTANCE_LIST,
            &json!({ "state": LIVE_STATES, "limit": MAX_LISTED }),
            None,
        )
        .await?;

    let builds = listed(&builds, "builds", "brr build list", |build| {
        format!("{} {}", text(build, "source"), text(build, "version"))
    });
    let profiles = listed(&profiles, "profiles", "brr profile list", |profile| {
        // The built-in one-shot Profile is the daemon's own; agents launch their own Profiles.
        if profile["builtin"] == true {
            return String::new();
        }

        format!("{} ({})", text(profile, "name"), text(profile, "kind"))
    });
    let instances = listed(&instances, "instances", "brr instance list", |instance| {
        format!(
            "{} ({}, tabs: {}) `{}`",
            text(instance, "profile_name"),
            text(instance, "state"),
            instance["tab_count"],
            text(instance, "id"),
        )
    });

    writeln!(out, "- Browsers installed: {builds}")?;
    writeln!(out, "- Profiles: {profiles}")?;
    writeln!(out, "- Open browsers: {instances}")?;

    Ok(format::data(&out)?)
}

fn listed(output: &Value, key: &str, more: &str, line: impl Fn(&Value) -> String) -> String {
    let items: Vec<String> = output[key]
        .as_array()
        .into_iter()
        .flatten()
        .map(line)
        .filter(|item| !item.is_empty())
        .collect();

    if items.is_empty() {
        return "none".to_string();
    }

    let joined = items.join(", ");
    if output["next_cursor"].is_string() {
        format!("{joined}, … (`{more}` lists all)")
    } else {
        joined
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("?")
}
