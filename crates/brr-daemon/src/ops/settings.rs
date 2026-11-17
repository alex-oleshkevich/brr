use std::collections::BTreeMap;

use brr_core::{
    Empty, Error, Http, Method, OperationSpec, SandboxMode, Secret, SecretStatus, Settings, Source,
    Sources, Toolset, schema,
};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use super::{OpRegistration, invalid, output, parse};
use crate::{
    app::Ctx,
    services::{backups::PASSPHRASE_SECRET, sandbox},
};

// The listener and the socket are bound once at start.
const RESTART_REQUIRED: [&str; 2] = ["server.bind", "server.socket"];
const SANDBOX_MODE: &str = "sandbox.mode";

static GET: OperationSpec = OperationSpec {
    name: "settings.get",
    group: "settings",
    toolset: Toolset::System,
    summary: "Read every setting and where each value comes from",
    http: Http {
        method: Method::Get,
        path: "/v1/settings",
    },
    cli: &["settings", "get"],
    cli_positional: &[],
    mcp_tool: Some("settings_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<SettingsOutput>,
};

static SET: OperationSpec = OperationSpec {
    name: "settings.set",
    group: "settings",
    toolset: Toolset::System,
    summary: "Change settings; the input maps `section.key` to its new value, null resets it",
    http: Http {
        method: Method::Patch,
        path: "/v1/settings",
    },
    cli: &["settings", "set"],
    cli_positional: &[],
    mcp_tool: Some("settings_set"),
    read_only: false,
    capability: None,
    input_schema: schema::<BTreeMap<String, Value>>,
    output_schema: schema::<SetOutput>,
};

inventory::submit!(OpRegistration {
    spec: &GET,
    handler: get
});
inventory::submit!(OpRegistration {
    spec: &SET,
    handler: set
});

#[derive(Serialize, JsonSchema)]
struct SettingsOutput {
    settings: Settings,
    sources: BTreeMap<String, String>,
    secrets: BTreeMap<String, SecretStatus>,
}

#[derive(Serialize, JsonSchema)]
struct SetOutput {
    settings: Settings,
    sources: BTreeMap<String, String>,
    restart_required: Vec<String>,
}

fn get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        let passphrase = ctx
            .app
            .secrets
            .status(&ctx.app.store, PASSPHRASE_SECRET)
            .await?;
        // The passphrase shows only as set or unset, never a character of it.
        let passphrase = SecretStatus {
            hint: None,
            ..passphrase
        };

        output(&SettingsOutput {
            settings: (*ctx.app.settings()).clone(),
            sources: source_names(&ctx.app.sources()),
            secrets: BTreeMap::from([(PASSPHRASE_SECRET.to_string(), passphrase)]),
        })
    }
    .boxed()
}

fn set(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let mut changes: BTreeMap<String, Value> = parse(input)?;

        // A secret lives in the vault, never in config.toml.
        let passphrase = changes
            .remove(PASSPHRASE_SECRET)
            .map(|value| {
                value
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .map(Secret::new)
                    .ok_or_else(|| invalid(PASSPHRASE_SECRET, "must be a non-empty string"))
            })
            .transpose()?;
        let restart_required = RESTART_REQUIRED
            .iter()
            .filter(|key| changes.contains_key(**key))
            .map(|key| key.to_string())
            .collect();

        let idle_changed = changes.keys().any(|key| key.starts_with("idle."));
        let capacity_changed = changes.keys().any(|key| {
            ["limits.", "adaptive."]
                .iter()
                .any(|section| key.starts_with(section))
        });

        // A sandbox that cannot start would fail every launch after it.
        let to_docker = changes
            .get(SANDBOX_MODE)
            .and_then(|value| serde_json::from_value::<SandboxMode>(value.clone()).ok())
            == Some(SandboxMode::Docker);
        if to_docker && let Err(message) = sandbox::probe().await {
            return Err(invalid(SANDBOX_MODE, &message));
        }

        if !changes.is_empty() {
            ctx.app.set_settings(changes).await?;
        }
        // Last, so a rejected patch never replaces the passphrase.
        if let Some(passphrase) = passphrase {
            ctx.app
                .secrets
                .set(&ctx.app.store, PASSPHRASE_SECRET, &passphrase)
                .await?;
        }
        if idle_changed {
            ctx.app.instances.recompute_idle().await;
        }
        if capacity_changed {
            ctx.app.admission.pump(&ctx.app).await;
        }

        output(&SetOutput {
            settings: (*ctx.app.settings()).clone(),
            sources: source_names(&ctx.app.sources()),
            restart_required,
        })
    }
    .boxed()
}

fn source_names(sources: &Sources) -> BTreeMap<String, String> {
    sources
        .iter()
        .map(|(key, source)| {
            let name = match source {
                Source::Default => "default",
                Source::File => "file",
                Source::Env => "env",
            };

            (key.clone(), name.to_string())
        })
        .collect()
}
