use std::collections::BTreeMap;

use brr_core::{
    AdmissionMode, Empty, Error, EventData, Http, Method, OperationSpec, Toolset, schema,
    settings::{
        AdaptiveSettings, AdmissionSettings, EngineLimits, LimitMode, LimitsSettings, ReserveMb,
    },
};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{MAX_PRIORITY, OpRegistration, invalid, output, parse};
use crate::{
    app::{App, Ctx},
    internal,
    services::admission::QueueEntry,
};

const MAX_LOAD_CEILING: f64 = 4.0;
const LIMITS_SECTION: &str = "limits";
const ADAPTIVE_SECTION: &str = "adaptive";
const ADMISSION_SECTION: &str = "admission";

static GET: OperationSpec = OperationSpec {
    name: "limits.get",
    group: "limits",
    toolset: Toolset::Instances,
    summary: "Show the instance and Tab limits, the adaptive thresholds and the admission defaults",
    http: Http {
        method: Method::Get,
        path: "/v1/limits",
    },
    cli: &["limits", "get"],
    cli_positional: &[],
    mcp_tool: Some("limits_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<LimitsView>,
};

static SET: OperationSpec = OperationSpec {
    name: "limits.set",
    group: "limits",
    toolset: Toolset::Instances,
    summary: "Change limits; only the given fields change and the file is rewritten",
    http: Http {
        method: Method::Put,
        path: "/v1/limits",
    },
    cli: &["limits", "set"],
    cli_positional: &[],
    mcp_tool: Some("limits_set"),
    read_only: false,
    capability: None,
    input_schema: schema::<SetInput>,
    output_schema: schema::<LimitsView>,
};

static QUEUE: OperationSpec = OperationSpec {
    name: "admission.queue",
    group: "admission",
    toolset: Toolset::Instances,
    summary: "List launches waiting for room, in the order they will be admitted",
    http: Http {
        method: Method::Get,
        path: "/v1/admission/queue",
    },
    cli: &["admission", "queue"],
    cli_positional: &[],
    mcp_tool: Some("admission_queue"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<QueueOutput>,
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
    spec: &QUEUE,
    handler: queue
});

#[derive(Serialize, JsonSchema)]
struct LimitsView {
    #[serde(flatten)]
    limits: LimitsSettings,
    adaptive: AdaptiveSettings,
    admission: AdmissionSettings,
}

#[derive(Serialize, JsonSchema)]
struct QueueOutput {
    queue: Vec<QueueEntry>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<LimitMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_instances: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_instances_per_engine: Option<EngineLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_instances_per_tag: Option<BTreeMap<String, u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tabs_per_instance: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tabs_total: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adaptive: Option<SetAdaptive>,
    #[serde(skip_serializing_if = "Option::is_none")]
    admission: Option<SetAdmission>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetAdaptive {
    #[serde(skip_serializing_if = "Option::is_none")]
    min_free_ram_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_load_per_core: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reserve_mb: Option<ReserveMb>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pressure_check_s: Option<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetAdmission {
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<AdmissionMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue_timeout_s: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<u8>,
}

fn limits_view(app: &App) -> LimitsView {
    let settings = app.settings();

    LimitsView {
        limits: settings.limits.clone(),
        adaptive: settings.adaptive.clone(),
        admission: settings.admission.clone(),
    }
}

fn validate(input: &SetInput) -> Result<(), Error> {
    if input.max_instances == Some(0) {
        return Err(invalid("max_instances", "must be at least 1"));
    }
    if input.max_tabs_per_instance == Some(0) {
        return Err(invalid("max_tabs_per_instance", "must be at least 1"));
    }
    if input
        .max_instances_per_engine
        .as_ref()
        .is_some_and(|engines| engines.chromium == 0 || engines.camoufox == 0)
    {
        return Err(invalid(
            "max_instances_per_engine",
            "chromium and camoufox must each be at least 1",
        ));
    }

    let load = input.adaptive.as_ref().and_then(|a| a.max_load_per_core);
    if load.is_some_and(|load| load <= 0.0 || load > MAX_LOAD_CEILING) {
        return Err(invalid(
            "adaptive.max_load_per_core",
            &format!("must be above 0 and at most {MAX_LOAD_CEILING}"),
        ));
    }

    let priority = input.admission.as_ref().and_then(|a| a.priority);
    if priority.is_some_and(|priority| priority > MAX_PRIORITY) {
        return Err(invalid(
            "admission.priority",
            &format!("must be between 0 and {MAX_PRIORITY}"),
        ));
    }

    Ok(())
}

// Settings keys are dotted; maps and structs below the key's own level stay whole values.
fn dotted_keys(input: &SetInput) -> Result<BTreeMap<String, Value>, Error> {
    let Value::Object(fields) = serde_json::to_value(input).map_err(internal)? else {
        return Err(internal("limits input is not an object"));
    };

    let mut keys = BTreeMap::new();
    for (name, value) in fields {
        match (name.as_str(), value) {
            (ADAPTIVE_SECTION | ADMISSION_SECTION, Value::Object(members)) => {
                for (member, value) in members {
                    keys.insert(format!("{name}.{member}"), value);
                }
            }
            (_, value) => {
                keys.insert(format!("{LIMITS_SECTION}.{name}"), value);
            }
        }
    }

    Ok(keys)
}

fn get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        output(&limits_view(&ctx.app))
    }
    .boxed()
}

fn set(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: SetInput = parse(input)?;
        validate(&input)?;

        let changes = dotted_keys(&input)?;
        let keys: Vec<String> = changes.keys().cloned().collect();
        ctx.app.set_settings(changes).await?;
        ctx.app
            .events
            .publish(EventData::LimitsChanged { keys })
            .await?;
        ctx.app.admission.pump(&ctx.app).await;

        output(&limits_view(&ctx.app))
    }
    .boxed()
}

fn queue(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        output(&QueueOutput {
            queue: ctx.app.admission.queue().await,
        })
    }
    .boxed()
}
