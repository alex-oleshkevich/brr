use std::time::Duration;

use brr_core::{
    ClientKind, Error, Http, HumanReason, Lease, Method, OperationSpec, Tab, TabId, Toolset, schema,
};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    OpRegistration, invalid, output, parse,
    tabs::{open_tab, tab_view},
    tool::{tab_call, tab_op},
};
use crate::app::Ctx;

const MAX_HUMAN_WAIT_MS: u64 = 3_600_000;
const MS_PER_S: u64 = 1000;

tab_op!(AWAIT_HUMAN {
    tool: "await_human",
    cli: "await-human",
    summary: "Wait until a person has done something in the Tab (solve a captcha, log in, close a dialog); fails with a timeout when nobody does",
    positional: &[],
    read_only: false,
    capability: None,
    input: AwaitInput,
    output: Tab,
    run: await_human,
});

// The three human overrides below are the web UI's: no MCP tool and no CLI command (cli: &[]).
static TAKE_OVER: OperationSpec = OperationSpec {
    name: "tab.take_over",
    group: "tab",
    toolset: Toolset::Browser,
    summary: "Take a Tab over from the Client that leases it, so a person can drive it",
    http: Http {
        method: Method::Post,
        path: "/v1/tabs/{tab_id}/take-over",
    },
    cli: &[],
    cli_positional: &[],
    mcp_tool: None,
    read_only: false,
    capability: None,
    input_schema: schema::<TabIdInput>,
    output_schema: schema::<Taken>,
};

static HAND_BACK: OperationSpec = OperationSpec {
    name: "tab.hand_back",
    group: "tab",
    toolset: Toolset::Browser,
    summary: "Give a Tab you took over back to the Client it came from",
    http: Http {
        method: Method::Post,
        path: "/v1/tabs/{tab_id}/hand-back",
    },
    cli: &[],
    cli_positional: &[],
    mcp_tool: None,
    read_only: false,
    capability: None,
    input_schema: schema::<TabIdInput>,
    output_schema: schema::<Taken>,
};

static HUMAN_DONE: OperationSpec = OperationSpec {
    name: "tab.human_done",
    group: "tab",
    toolset: Toolset::Browser,
    summary: "Tell a Tab that is awaiting a person that the person is done",
    http: Http {
        method: Method::Post,
        path: "/v1/tabs/{tab_id}/human-done",
    },
    cli: &[],
    cli_positional: &[],
    mcp_tool: None,
    read_only: false,
    capability: None,
    input_schema: schema::<TabIdInput>,
    output_schema: schema::<Tab>,
};

inventory::submit!(OpRegistration {
    spec: &TAKE_OVER,
    handler: take_over
});
inventory::submit!(OpRegistration {
    spec: &HAND_BACK,
    handler: hand_back
});
inventory::submit!(OpRegistration {
    spec: &HUMAN_DONE,
    handler: human_done
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AwaitInput {
    tab_id: Option<TabId>,
    reason: HumanReason,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TabIdInput {
    tab_id: TabId,
}

#[derive(Serialize, JsonSchema)]
struct Taken {
    tab: Tab,
    lease: Lease,
}

async fn await_human(ctx: Ctx, input: AwaitInput) -> Result<Value, Error> {
    let timeout_ms = match input.timeout_ms {
        Some(0) => return Err(invalid("timeout_ms", "must be at least 1")),
        Some(timeout_ms) if timeout_ms > MAX_HUMAN_WAIT_MS => {
            return Err(invalid(
                "timeout_ms",
                &format!("must be at most {MAX_HUMAN_WAIT_MS}"),
            ));
        }
        Some(timeout_ms) => timeout_ms,
        None => ctx
            .app
            .settings()
            .captcha
            .human_wait_s
            .saturating_mul(MS_PER_S)
            .min(MAX_HUMAN_WAIT_MS),
    };

    let call = tab_call(&ctx, input.tab_id, &AWAIT_HUMAN).await?;
    let tab = ctx
        .app
        .instances
        .await_human(
            &call.instance_id,
            &call.tab_id,
            input.reason,
            Duration::from_millis(timeout_ms),
        )
        .await?;

    output(&tab_view(&ctx.app, tab).await)
}

// Only the UI Client: a REST or CLI caller would take another agent's Lease, and its implicit
// Client ends with the call, releasing the Lease it took.
fn ui_only(ctx: &Ctx) -> Result<(), Error> {
    if ctx.client.kind == ClientKind::Ui {
        return Ok(());
    }

    Err(invalid(
        "client",
        "only the web UI may take over a Tab, hand it back or finish a human step",
    ))
}

// No Lease guard: the person overrides the Client that holds it (LC6).
fn take_over(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: TabIdInput = parse(input)?;
        ui_only(&ctx)?;
        let stored = open_tab(&ctx, Some(input.tab_id)).await?;

        let lease = ctx
            .app
            .leases
            .take_over(&ctx.app, &ctx.client, &stored.instance_id, &stored.id)
            .await?;
        ctx.app
            .clients
            .set_current_tab(&ctx.app, &ctx.client.client_id, Some(&stored.id))
            .await?;

        output(&Taken {
            tab: tab_view(&ctx.app, stored).await,
            lease,
        })
    }
    .boxed()
}

fn hand_back(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: TabIdInput = parse(input)?;
        ui_only(&ctx)?;
        let stored = open_tab(&ctx, Some(input.tab_id)).await?;

        let lease = ctx
            .app
            .leases
            .hand_back(&ctx.app, &ctx.client, &stored.id)
            .await?;

        output(&Taken {
            tab: tab_view(&ctx.app, stored).await,
            lease,
        })
    }
    .boxed()
}

// No Lease guard either: the person in the UI signals it, not the Client that holds the Tab.
fn human_done(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: TabIdInput = parse(input)?;
        ui_only(&ctx)?;
        let stored = open_tab(&ctx, Some(input.tab_id)).await?;

        let tab = ctx
            .app
            .instances
            .human_done(&ctx.app, &stored.instance_id, &stored.id)
            .await?;

        output(&tab_view(&ctx.app, tab).await)
    }
    .boxed()
}
