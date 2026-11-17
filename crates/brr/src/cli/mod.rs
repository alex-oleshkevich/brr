use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::bail;
use brr_core::ClientKind;
use brr_daemon::{
    Boot,
    ops::{self, OpRegistration},
};
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::{Value, json};

mod archive;
mod args;
mod daemon;
mod doctor;
mod files;
pub mod format;
mod mcp;
mod prime;
mod render;
mod runner;
mod skill;
mod systemd;

#[cfg(test)]
mod tests;

pub const MCP: &str = mcp::NAME;
pub const DAEMON: &str = "daemon";
pub const JSON: &str = "json";
pub const QUIET: &str = "quiet";
pub const VERBOSE: &str = "verbose";

const VERSION: &str = "version";
const FOLLOW: &str = "follow";
const EVENTS_LIST: &str = "events.list";
const ARTIFACT_FETCH: &str = "artifact.fetch";
const CLIENT_END: &str = "client.end";
const REAPPLY_TEMPLATE: &str = "profile.reapply_template";
const TEMPLATE_DELETE: &str = "template.delete";
const DRY_RUN: &str = "dry_run";
const SETTINGS_SET: &str = "settings.set";
const RESTART_REQUIRED: &str = "restart_required";

const STREAM_QUERY_FIELDS: [&str; 3] = ["since", "kinds", "subject"];
// The stream endpoint filters by these three only.
const FOLLOW_CONFLICTS: [&str; 6] = [
    "instance_id",
    "from_ts",
    "to_ts",
    "order",
    "limit",
    "cursor",
];

#[derive(Default)]
struct Node {
    op: Option<&'static OpRegistration>,
    children: BTreeMap<&'static str, Node>,
}

pub fn command() -> Command {
    let mut tree = Node::default();
    // An op without CLI words is the web UI's alone.
    for op in ops::all().into_iter().filter(|op| !op.spec.cli.is_empty()) {
        let node = op.spec.cli.iter().fold(&mut tree, |node, word| {
            node.children.entry(word).or_default()
        });
        node.op = Some(op);
    }

    let root = Command::new("brr")
        .about("Install browsers, run them from profiles and track their tabs")
        .styles(format::STYLES)
        .subcommand_required(true)
        .arg_required_else_help(true)
        .arg(
            Arg::new(JSON)
                .long(JSON)
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Print the typed output as one JSON line"),
        )
        .arg(
            Arg::new(QUIET)
                .short('q')
                .long(QUIET)
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Log less"),
        )
        .arg(
            Arg::new(VERBOSE)
                .short('v')
                .long(VERBOSE)
                .global(true)
                .action(ArgAction::Count)
                .help("Log more; twice for trace"),
        )
        .subcommand(daemon::command())
        .subcommand(mcp::command())
        .subcommand(runner::command())
        .subcommand(Command::new(doctor::NAME).about("Check this machine and the daemon's setup"))
        .subcommand(
            Command::new(prime::NAME)
                .about("Print the agent guide and what runs right now; run it at session start"),
        )
        .subcommand(skill::command())
        .subcommand(Command::new(VERSION).about("Print the version"));

    tree.children.into_iter().fold(root, |root, (name, node)| {
        root.subcommand(node_command(name, node))
    })
}

fn node_command(name: &'static str, node: Node) -> Command {
    let mut command = Command::new(name);

    if let Some(op) = node.op {
        command = args::add(command.about(op.spec.summary), op.spec);

        if op.spec.name == EVENTS_LIST {
            command = command.arg(
                Arg::new(FOLLOW)
                    .long(FOLLOW)
                    .action(ArgAction::SetTrue)
                    .conflicts_with_all(FOLLOW_CONFLICTS)
                    .help("Stream new events after --since as they happen"),
            );
        }
    }

    if node.children.is_empty() {
        return command;
    }

    command = if node.op.is_some() {
        command.args_conflicts_with_subcommands(true)
    } else {
        command
            .subcommand_required(true)
            .arg_required_else_help(true)
    };

    node.children
        .into_iter()
        .fold(command, |command, (name, node)| {
            command.subcommand(node_command(name, node))
        })
}

// The innermost subcommand and the op whose CLI path leads to it.
fn leaf_op(matches: &ArgMatches) -> Option<(&'static OpRegistration, &ArgMatches)> {
    let mut path = Vec::new();
    let mut leaf = matches;
    while let Some((name, sub)) = leaf.subcommand() {
        path.push(name);
        leaf = sub;
    }

    let op = ops::all().into_iter().find(|op| op.spec.cli == path)?;

    Some((op, leaf))
}

pub async fn run(boot: Boot, matches: &ArgMatches, argv0: &str) -> anyhow::Result<()> {
    let json = matches.get_flag(JSON);

    match matches.subcommand() {
        Some((VERSION, _)) => Ok(format::data(boot.version)?),
        Some((doctor::NAME, _)) => doctor::run(&boot, json).await,
        Some((prime::NAME, _)) => prime::run(&boot).await,
        Some((skill::NAME, skill)) => skill::run(skill),
        Some((MCP, mcp)) => mcp::run(&boot, mcp).await,
        Some((DAEMON, daemon)) => daemon::run(boot, daemon, json).await,
        Some((runner::NAME, _)) => runner::run().await,
        _ => match leaf_op(matches) {
            Some((op, leaf)) => run_op(&boot, argv0, op, leaf, json).await,
            None => bail!("unknown command"),
        },
    }
}

async fn run_op(
    boot: &Boot,
    argv0: &str,
    op: &OpRegistration,
    matches: &ArgMatches,
    json: bool,
) -> anyhow::Result<()> {
    let spec = op.spec;
    if let Some(done) = archive::run(boot, argv0, spec, matches, json).await? {
        return Ok(done);
    }

    let input = args::input(spec, matches)?;
    let file = args::output_file(spec);
    let path = file
        .as_ref()
        .and_then(|_| matches.get_one::<PathBuf>(args::OUTPUT))
        .map(PathBuf::as_path);
    if let Some(file) = &file {
        render::require_path(file, path, json)?;
    }
    let remote = daemon::ensure_daemon(&boot.paths, &boot.settings).await?;

    if spec.name == EVENTS_LIST && matches.get_flag(FOLLOW) {
        return follow(&remote, &input).await;
    }

    let output = if spec.read_only {
        remote.call(spec.name, &input, None).await?
    } else {
        let label = format!("{argv0} {}", spec.cli.join(" "));

        as_client(&remote, &label, async |client_id| {
            remote.call(spec.name, &input, Some(client_id)).await
        })
        .await?
    };

    let output = inline_artifact(&remote, output, file.as_ref(), path).await?;
    if !json && let Some((verb, text)) = summary(spec.name, &input, &output) {
        return Ok(format::status(verb, &text)?);
    }
    if !json && spec.name == SETTINGS_SET {
        let changed = changed_settings(&input, &output);
        let fields: Vec<(&str, String)> = changed
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();

        return Ok(format::record(&fields)?);
    }

    render::output(&output, json, file.as_ref(), path)
}

// A one-line answer where the whole output would only repeat what the command asked for.
fn summary(op: &str, input: &Value, output: &Value) -> Option<(&'static str, String)> {
    let count = |value: &Value| value.as_array().map_or(0, Vec::len);

    match op {
        REAPPLY_TEMPLATE if input[DRY_RUN] != json!(true) => {
            let changes = count(&output["changes"])
                + count(&output["extensions"]["added"])
                + count(&output["extensions"]["removed"]);
            let noun = if changes == 1 { "change" } else { "changes" };

            Some(("Applied", format!("{changes} {noun}")))
        }
        TEMPLATE_DELETE => Some((
            "Deleted",
            format!("template {}", output["name"].as_str().unwrap_or_default()),
        )),
        _ => None,
    }
}

// The keys the command set with their new values, not the whole settings object.
fn changed_settings(input: &Value, output: &Value) -> Vec<(String, String)> {
    let text = |value: &Value| match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let mut fields: Vec<(String, String)> = input
        .as_object()
        .into_iter()
        .flat_map(|changes| changes.keys())
        .map(|key| {
            let pointer = format!("/settings/{}", key.replace('.', "/"));

            (
                key.clone(),
                output.pointer(&pointer).map(text).unwrap_or_default(),
            )
        })
        .collect();
    if output[RESTART_REQUIRED]
        .as_array()
        .is_some_and(|keys| !keys.is_empty())
    {
        fields.push((
            RESTART_REQUIRED.to_string(),
            text(&output[RESTART_REQUIRED]),
        ));
    }

    fields
}

// The presence connection releases this Client's Leases if the process dies mid-call.
async fn as_client<T>(
    remote: &brr_daemon::remote::Remote,
    label: &str,
    call: impl AsyncFnOnce(&brr_core::ClientId) -> Result<T, brr_core::Error>,
) -> anyhow::Result<T> {
    let client = remote.create_client(ClientKind::Cli, label).await?;
    let _hold = remote.hold(&client.id).await?;

    let result = call(&client.id).await;
    if let Err(error) = remote
        .call(CLIENT_END, &json!({ "client_id": client.id }), None)
        .await
    {
        format::warning(&format!("could not end the client: {error}"))?;
    }

    Ok(result?)
}

// A result the daemon answered with a URL has no bytes to write to -o; the Artifact's bytes
// come from artifact.fetch instead, so the CLI needs no HTTP client of its own.
async fn inline_artifact(
    remote: &brr_daemon::remote::Remote,
    mut output: Value,
    file: Option<&args::OutputFile>,
    path: Option<&Path>,
) -> anyhow::Result<Value> {
    let (Some(file), Some(_)) = (file, path) else {
        return Ok(output);
    };
    let Some(artifact_id) = output
        .get("artifact_id")
        .and_then(Value::as_str)
        .filter(|_| output.get(&file.field).is_none_or(Value::is_null))
    else {
        return Ok(output);
    };

    let input = json!({ "artifact_id": artifact_id, "return": "inline" });
    let mut fetched = remote.call(ARTIFACT_FETCH, &input, None).await?;
    output[&file.field] = fetched[&file.field].take();

    Ok(output)
}

async fn follow(remote: &brr_daemon::remote::Remote, input: &Value) -> anyhow::Result<()> {
    let query: Vec<(&str, String)> = STREAM_QUERY_FIELDS
        .iter()
        .filter_map(|field| {
            let value = input.get(field)?;
            let text = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_string);

            Some((*field, text))
        })
        .collect();

    let mut events = remote.follow_events(&query).await?;
    while let Some(event) = events.recv().await {
        format::data(&serde_json::to_string(&event?)?)?;
    }

    Ok(())
}
