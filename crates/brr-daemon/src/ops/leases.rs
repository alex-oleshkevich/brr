use brr_core::{
    ClientId, Error, Http, InstanceId, Lease, LeaseId, Method, OperationSpec, Page, TabId,
    TabState, Toolset, schema,
};
use brr_store::{lease, tab};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, ensure_live, invalid, output, parse};
use crate::app::Ctx;

const CURSOR_SEPARATOR: char = ':';

static LIST: OperationSpec = OperationSpec {
    name: "lease.list",
    group: "lease",
    toolset: Toolset::Instances,
    summary: "List active Leases, oldest first; include_released adds the released ones, for example to see who used a finished Instance",
    http: Http {
        method: Method::Get,
        path: "/v1/leases",
    },
    cli: &["lease", "list"],
    cli_positional: &[],
    mcp_tool: Some("lease_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ListOutput>,
};

static ACQUIRE: OperationSpec = OperationSpec {
    name: "lease.acquire",
    group: "lease",
    toolset: Toolset::Instances,
    summary: "Lease a Tab, or the whole Instance with `exclusive`, so other Clients cannot change it",
    http: Http {
        method: Method::Post,
        path: "/v1/leases",
    },
    cli: &["lease", "acquire"],
    cli_positional: &[],
    mcp_tool: Some("lease_acquire"),
    read_only: false,
    capability: None,
    input_schema: schema::<AcquireInput>,
    output_schema: schema::<Lease>,
};

static RELEASE: OperationSpec = OperationSpec {
    name: "lease.release",
    group: "lease",
    toolset: Toolset::Instances,
    summary: "Release a Lease you hold",
    http: Http {
        method: Method::Delete,
        path: "/v1/leases/{lease_id}",
    },
    cli: &["lease", "release"],
    cli_positional: &["lease_id"],
    mcp_tool: Some("lease_release"),
    read_only: false,
    capability: None,
    input_schema: schema::<ReleaseInput>,
    output_schema: schema::<Lease>,
};

inventory::submit!(OpRegistration {
    spec: &LIST,
    handler: list
});
inventory::submit!(OpRegistration {
    spec: &ACQUIRE,
    handler: acquire
});
inventory::submit!(OpRegistration {
    spec: &RELEASE,
    handler: release
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    instance_id: Option<InstanceId>,
    client_id: Option<ClientId>,
    #[serde(default)]
    include_released: bool,
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    leases: Vec<Lease>,
    next_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AcquireInput {
    instance_id: InstanceId,
    tab_id: Option<TabId>,
    exclusive: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReleaseInput {
    lease_id: LeaseId,
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let page = Page::new(input.limit, input.cursor);
        let after = page.cursor.as_deref().map(parse_cursor).transpose()?;

        let mut leases = if input.include_released {
            let limit = page.limit + 1;
            ctx.app
                .store
                .run(move |c| {
                    lease::history(
                        c,
                        input.instance_id.as_ref(),
                        input.client_id.as_ref(),
                        after,
                        limit,
                    )
                })
                .await?
        } else {
            let mut leases = ctx
                .app
                .leases
                .list(input.instance_id.as_ref(), input.client_id.as_ref())
                .await;
            leases.sort_by_key(|l| (l.acquired_at, l.id));
            leases.retain(|l| after.is_none_or(|after| (l.acquired_at, l.id) > after));

            leases
        };

        let limit = page.limit as usize;
        let has_more = leases.len() > limit;
        leases.truncate(limit);
        let next_cursor = match leases.last() {
            Some(last) if has_more => {
                Some(format!("{}{CURSOR_SEPARATOR}{}", last.acquired_at, last.id))
            }
            _ => None,
        };

        output(&ListOutput {
            leases,
            next_cursor,
        })
    }
    .boxed()
}

fn acquire(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: AcquireInput = parse(input)?;
        let exclusive = input.exclusive.unwrap_or(false);
        let tab_id = match (input.tab_id, exclusive) {
            (Some(tab_id), false) => tab_id,
            (None, true) => return acquire_exclusive(ctx, input.instance_id).await,
            (Some(_), true) => {
                return Err(invalid(
                    "exclusive",
                    "an exclusive Lease covers the whole Instance; drop tab_id",
                ));
            }
            (None, false) => {
                return Err(invalid(
                    "tab_id",
                    "give a tab_id, or set exclusive to lease the whole Instance",
                ));
            }
        };

        let instance_id = input.instance_id;
        ensure_live(&ctx, instance_id).await?;
        let tab = ctx.app.store.run(move |c| tab::get(c, &tab_id)).await?;
        if tab.state == TabState::Closed || tab.instance_id != instance_id {
            return Err(Error::NotFound {
                what: format!("tab {tab_id}"),
            });
        }

        let lease = ctx
            .app
            .leases
            .acquire_tab(&ctx.app, &ctx.client, &instance_id, &tab_id)
            .await?;

        output(&lease)
    }
    .boxed()
}

async fn acquire_exclusive(ctx: Ctx, instance_id: InstanceId) -> Result<Value, Error> {
    ensure_live(&ctx, instance_id).await?;

    let lease = ctx
        .app
        .leases
        .acquire_exclusive(&ctx.app, &ctx.client, &instance_id)
        .await?;

    // The wait can be long; the Instance may have stopped meanwhile.
    if let Err(error) = ensure_live(&ctx, instance_id).await {
        // The instance error is the answer; a failed undo is only logged.
        let undone = ctx
            .app
            .leases
            .release(&ctx.app, &ctx.client, &lease.id)
            .await;
        if let Err(undo_error) = undone {
            tracing::warn!(lease_id = %lease.id, %undo_error, "exclusive lease undo failed");
        }

        return Err(error);
    }

    output(&lease)
}

fn release(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ReleaseInput = parse(input)?;
        let lease = ctx
            .app
            .leases
            .release(&ctx.app, &ctx.client, &input.lease_id)
            .await?;

        output(&lease)
    }
    .boxed()
}

fn parse_cursor(cursor: &str) -> Result<(i64, LeaseId), Error> {
    let parsed = cursor
        .split_once(CURSOR_SEPARATOR)
        .and_then(|(at, id)| Some((at.parse().ok()?, id.parse().ok()?)));

    parsed.ok_or_else(|| {
        invalid(
            "cursor",
            "not a cursor from a previous page; pass next_cursor from the previous page, or omit cursor for the first page",
        )
    })
}
