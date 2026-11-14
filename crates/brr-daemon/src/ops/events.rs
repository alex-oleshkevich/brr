use std::time::Duration;

use brr_core::{
    Error, Event, Http, InstanceId, Method, OperationSpec, Order, Page, Toolset, schema,
};
use brr_store::event::{self, Query};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{sync::broadcast::error::RecvError, time};

use super::{OpRegistration, output, parse};
use crate::{
    app::{App, Ctx},
    services::events::{parse_subject, split_kinds},
};

const WATCH_MAX_TIMEOUT_MS: u64 = 30_000;
const WATCH_PAGE_LIMIT: u32 = 1000;

static LIST: OperationSpec = OperationSpec {
    name: "events.list",
    group: "events",
    toolset: Toolset::System,
    summary: "List persisted events, newest first, or oldest first after `since`",
    http: Http {
        method: Method::Get,
        path: "/v1/events",
    },
    cli: &["events"],
    cli_positional: &[],
    mcp_tool: Some("events_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ListOutput>,
};

static WATCH: OperationSpec = OperationSpec {
    name: "events.watch",
    group: "events",
    toolset: Toolset::System,
    summary: "Wait for events after `since`; returns early on the first match, empty on timeout",
    http: Http {
        method: Method::Get,
        path: "/v1/events/watch",
    },
    cli: &["events", "watch"],
    cli_positional: &[],
    mcp_tool: Some("events_watch"),
    read_only: true,
    capability: None,
    input_schema: schema::<WatchInput>,
    output_schema: schema::<WatchOutput>,
};

inventory::submit!(OpRegistration {
    spec: &LIST,
    handler: list
});
inventory::submit!(OpRegistration {
    spec: &WATCH,
    handler: watch
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    since: Option<i64>,
    kinds: Option<String>,
    subject: Option<String>,
    instance_id: Option<InstanceId>,
    from_ts: Option<i64>,
    to_ts: Option<i64>,
    order: Option<Order>,
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    events: Vec<Event>,
    next_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WatchInput {
    since: Option<i64>,
    kinds: Option<String>,
    timeout_ms: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
struct WatchOutput {
    events: Vec<Event>,
    next_since: i64,
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let query = Query {
            since: input.since,
            kinds: split_kinds(input.kinds.as_deref()),
            subject: input.subject.as_deref().map(parse_subject).transpose()?,
            instance_id: input.instance_id,
            from_ts: input.from_ts,
            to_ts: input.to_ts,
            order: input.order.unwrap_or(Order::Desc),
            page: Page::new(input.limit, input.cursor),
        };

        let (events, next_cursor) = ctx.app.store.run(move |c| event::list(c, &query)).await?;

        output(&ListOutput {
            events,
            next_cursor,
        })
    }
    .boxed()
}

fn watch(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: WatchInput = parse(input)?;
        let kinds = split_kinds(input.kinds.as_deref());
        let timeout = Duration::from_millis(
            input
                .timeout_ms
                .unwrap_or(WATCH_MAX_TIMEOUT_MS)
                .min(WATCH_MAX_TIMEOUT_MS),
        );
        let deadline = time::Instant::now() + timeout;

        // Subscribing before the DB read leaves no gap between the two.
        let mut rx = ctx.app.events.subscribe();
        let since = match input.since {
            Some(since) => since,
            None => ctx.app.events.last_seq().await,
        };

        let mut events = persisted_since(&ctx.app, since, &kinds).await?;

        while events.is_empty() {
            match time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(published)) => {
                    let event = &published.event;
                    if event.seq > since && (kinds.is_empty() || kinds.contains(&event.kind)) {
                        events.push(event.clone());
                    }
                }
                // Events were dropped from the buffer; they may still be in the table.
                Ok(Err(RecvError::Lagged(_))) => {
                    events = persisted_since(&ctx.app, since, &kinds).await?;
                }
                Ok(Err(RecvError::Closed)) | Err(_) => break,
            }
        }

        let next_since = events.last().map_or(since, |event| event.seq);

        output(&WatchOutput { events, next_since })
    }
    .boxed()
}

async fn persisted_since(app: &App, since: i64, kinds: &[String]) -> Result<Vec<Event>, Error> {
    let query = Query {
        since: Some(since),
        kinds: kinds.to_vec(),
        subject: None,
        instance_id: None,
        from_ts: None,
        to_ts: None,
        order: Order::Asc,
        page: Page::new(Some(WATCH_PAGE_LIMIT), None),
    };

    let (events, _) = app.store.run(move |c| event::list(c, &query)).await?;

    Ok(events)
}
