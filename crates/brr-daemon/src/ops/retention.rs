use brr_core::{
    Empty, Error, Http, Method, OperationSpec, Toolset, schema, settings::RetentionSettings,
};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, invalid, output, parse};
use crate::{
    app::Ctx,
    services::retention::{self, SweepReport},
};

const KEY_PREFIX: &str = "retention.";

static GET: OperationSpec = OperationSpec {
    name: "retention.get",
    group: "retention",
    toolset: Toolset::Storage,
    summary: "Show the global retention settings (Profiles can override the Artifact ones)",
    http: Http {
        method: Method::Get,
        path: "/v1/retention",
    },
    cli: &["retention", "get"],
    cli_positional: &[],
    mcp_tool: Some("retention_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<RetentionSettings>,
};

static SET: OperationSpec = OperationSpec {
    name: "retention.set",
    group: "retention",
    toolset: Toolset::Storage,
    summary: "Change global retention settings; omitted keys stay as they are",
    http: Http {
        method: Method::Put,
        path: "/v1/retention",
    },
    cli: &["retention", "set"],
    cli_positional: &[],
    mcp_tool: Some("retention_set"),
    read_only: false,
    capability: None,
    input_schema: schema::<SetInput>,
    output_schema: schema::<RetentionSettings>,
};

static SWEEP: OperationSpec = OperationSpec {
    name: "retention.sweep",
    group: "retention",
    toolset: Toolset::Storage,
    summary: "Delete what retention says is due; dry_run lists exactly what a real sweep would delete",
    http: Http {
        method: Method::Post,
        path: "/v1/retention/sweep",
    },
    cli: &["retention", "sweep"],
    cli_positional: &[],
    mcp_tool: Some("retention_sweep"),
    read_only: false,
    capability: None,
    input_schema: schema::<SweepInput>,
    output_schema: schema::<SweepReport>,
};

inventory::submit!(OpRegistration {
    spec: &GET,
    handler: get
});
inventory::submit!(OpRegistration {
    spec: &SET,
    handler: set
});
inventory::submit!(OpRegistration {
    spec: &SWEEP,
    handler: sweep
});

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    artifacts_max_age_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifacts_max_total_gb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    download_max_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    free_disk_floor_gb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unused_days: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    events_days: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SweepInput {
    dry_run: Option<bool>,
}

fn get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        output(&ctx.app.settings().retention)
    }
    .boxed()
}

fn set(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: SetInput = parse(input)?;
        if input.download_max_mb == Some(0) {
            return Err(invalid("download_max_mb", "must be at least 1"));
        }

        let Value::Object(fields) = serde_json::to_value(&input).map_err(crate::internal)? else {
            return Err(crate::internal("retention input is not an object"));
        };
        let changes = fields
            .into_iter()
            .map(|(key, value)| (format!("{KEY_PREFIX}{key}"), value))
            .collect();
        ctx.app.set_settings(changes).await?;

        output(&ctx.app.settings().retention)
    }
    .boxed()
}

fn sweep(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: SweepInput = parse(input)?;

        output(&retention::sweep(&ctx.app, input.dry_run.unwrap_or(false)).await?)
    }
    .boxed()
}
