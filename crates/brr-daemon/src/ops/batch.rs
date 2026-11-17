use std::time::{Duration, Instant};

use brr_core::{Error, ErrorBody, Http, Method, OperationSpec, Toolset, schema};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{OpRegistration, dispatch, find_mcp, invalid, output, parse};
use crate::app::Ctx;

const MAX_STEPS: usize = 50;
// Checked between steps: cutting a step off mid-flight would leave the page in an unknown state.
const DEADLINE: Duration = Duration::from_secs(120);
// Past this much output in one response, later steps still run but return only their size.
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const CLIENT_ID_ARG: &str = "client_id";
const BATCH_OP: &str = "batch.run";

static RUN: OperationSpec = OperationSpec {
    name: BATCH_OP,
    group: "batch",
    toolset: Toolset::Browser,
    summary: "Run several Browser tools in order in one call. Use it only for steps you can plan without seeing the page in between: refs from `snapshot` go stale after any navigation or page change, so address elements by `selector`, and pass `tab_id` on every step (without a `client_id`, `tab_open` does not make its Tab current for later steps). Only tool names are checked before the first step; a wrong argument fails when its step runs, after earlier steps have been applied. It stops at the first error, or after about two minutes between steps; later steps are not run and not listed, so compare `completed` (steps that ran) with `total`, and check `failed`. A rejected request is a tool error, a failed step is a result with `ok: false`. Never batch a step whose input depends on an earlier step's output.",
    http: Http {
        method: Method::Post,
        path: "/v1/batch",
    },
    cli: &["batch", "run"],
    cli_positional: &[],
    mcp_tool: Some("batch_run"),
    read_only: false,
    capability: None,
    input_schema: schema::<BatchInput>,
    output_schema: schema::<BatchOutput>,
};

inventory::submit!(OpRegistration {
    spec: &RUN,
    handler: run
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BatchInput {
    #[schemars(
        description = "1 to 50 steps, run in order. Each is a Browser tool name and its arguments."
    )]
    steps: Vec<Step>,
    #[serde(default)]
    #[schemars(description = "Keep going after a failed step. Default: stop at the first error.")]
    continue_on_error: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Step {
    #[schemars(
        description = "A Browser tool name as listed by tools/list, for example `navigate`, `click`, `fill` or `get_metadata`."
    )]
    tool: String,
    #[serde(default)]
    #[schemars(
        description = "The tool's own arguments. Set `tab_id`; `client_id` is not allowed here, pass it on batch_run."
    )]
    args: Map<String, Value>,
}

#[derive(Serialize, JsonSchema)]
struct BatchOutput {
    total: usize,
    #[schemars(description = "Steps that ran, failed ones included.")]
    completed: usize,
    #[schemars(description = "Steps that ran and failed.")]
    failed: usize,
    results: Vec<StepResult>,
    stopped: Option<Stopped>,
}

#[derive(Serialize, JsonSchema)]
struct StepResult {
    step: usize,
    tool: String,
    ok: bool,
    output: Option<Value>,
    error: Option<ErrorBody>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Stopped {
    Error,
    Deadline,
}

// Resolves every tool before the first step runs. Arguments are parsed by each tool when its
// step runs, so a bad argument surfaces at that step.
fn resolve(steps: Vec<Step>) -> Result<Vec<(&'static OpRegistration, Step)>, Error> {
    steps
        .into_iter()
        .enumerate()
        .map(|(index, step)| {
            let field = format!("steps[{index}]");
            let op = find_mcp(&step.tool).ok_or_else(|| {
                invalid(
                    &format!("{field}.tool"),
                    "unknown tool; use a Browser tool such as navigate, click or fill",
                )
            })?;
            if op.spec.toolset != Toolset::Browser || op.spec.name == BATCH_OP {
                return Err(invalid(
                    &format!("{field}.tool"),
                    "only Browser tools can run in a batch, and not batch_run itself",
                ));
            }
            if step.args.contains_key(CLIENT_ID_ARG) {
                return Err(invalid(
                    &format!("{field}.args.{CLIENT_ID_ARG}"),
                    "every step runs as the batch's Client; pass client_id on batch_run",
                ));
            }

            Ok((op, step))
        })
        .collect()
}

fn capped(value: Value, spent: &mut usize) -> Value {
    let size = value.to_string().len();
    if *spent + size > MAX_OUTPUT_BYTES {
        return json!({ "truncated": true, "bytes": size });
    }
    *spent += size;

    value
}

fn run(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: BatchInput = parse(input)?;
        if input.steps.is_empty() || input.steps.len() > MAX_STEPS {
            return Err(invalid(
                "steps",
                &format!("give between 1 and {MAX_STEPS} steps"),
            ));
        }

        let steps = resolve(input.steps)?;
        let total = steps.len();
        let started = Instant::now();
        let mut results = Vec::with_capacity(total);
        let mut failed = 0;
        let mut stopped = None;
        let mut spent = 0;

        for (step_index, (op, step)) in steps.into_iter().enumerate() {
            if started.elapsed() >= DEADLINE {
                stopped = Some(Stopped::Deadline);
                break;
            }

            match dispatch(ctx.clone(), op, Value::Object(step.args)).await {
                Ok(value) => {
                    results.push(StepResult {
                        step: step_index,
                        tool: step.tool,
                        ok: true,
                        output: Some(capped(value, &mut spent)),
                        error: None,
                    });
                }
                Err(error) => {
                    failed += 1;
                    results.push(StepResult {
                        step: step_index,
                        tool: step.tool,
                        ok: false,
                        output: None,
                        error: Some(error.body()),
                    });
                    if !input.continue_on_error {
                        stopped = Some(Stopped::Error);
                        break;
                    }
                }
            }
        }

        let completed = results.len();
        tracing::info!(
            steps = total,
            completed,
            failed,
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "batch"
        );

        output(&BatchOutput {
            total,
            completed,
            failed,
            results,
            stopped,
        })
    }
    .boxed()
}
