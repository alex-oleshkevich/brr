use std::{collections::HashMap, sync::Arc};

use brr_core::{Error, Http, Method, OperationSpec, Toolset, now_ms, schema};
use brr_store::{
    instance::{self, TrafficGroup, TrafficTotal},
    proxy,
};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, invalid, output, parse};
use crate::{
    app::{App, Ctx},
    services::proxies,
};

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
const DEFAULT_TRAFFIC_DAYS: u32 = 7;
const MAX_TRAFFIC_DAYS: u32 = 90;
const DAY_BUCKETS: i64 = 24;
const WEEK_BUCKETS: i64 = 28;
const WEEK_BUCKET_HOURS: i64 = 6;
const ATTACHED: &str = "attached";
const CAMOUFOX: &str = "camoufox";

static HISTORY: OperationSpec = OperationSpec {
    name: "usage.history",
    group: "usage",
    toolset: Toolset::Instances,
    summary: "Peak number of browsers running at once in each hour of the last day, or each 6 hours of the last week, by engine",
    http: Http {
        method: Method::Get,
        path: "/v1/usage/history",
    },
    cli: &["usage", "history"],
    cli_positional: &[],
    mcp_tool: Some("usage_history"),
    read_only: true,
    capability: None,
    input_schema: schema::<HistoryInput>,
    output_schema: schema::<HistoryOutput>,
};

inventory::submit!(OpRegistration {
    spec: &HISTORY,
    handler: history
});

static TRAFFIC: OperationSpec = OperationSpec {
    name: "usage.traffic",
    group: "usage",
    toolset: Toolset::Instances,
    summary: "Bytes sent and received through proxies over the last days, per Profile or per Proxy; runs without a forwarder are not measured",
    http: Http {
        method: Method::Get,
        path: "/v1/usage/traffic",
    },
    cli: &["usage", "traffic"],
    cli_positional: &[],
    mcp_tool: Some("usage_traffic"),
    read_only: true,
    capability: None,
    input_schema: schema::<TrafficInput>,
    output_schema: schema::<TrafficOutput>,
};

inventory::submit!(OpRegistration {
    spec: &TRAFFIC,
    handler: traffic
});

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
enum UsageWindow {
    #[serde(rename = "1d")]
    Day,
    #[serde(rename = "7d")]
    Week,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HistoryInput {
    window: Option<UsageWindow>,
}

#[derive(Serialize, JsonSchema)]
struct HistoryOutput {
    window: UsageWindow,
    bucket_ms: i64,
    buckets: Vec<UsageBucket>,
}

#[derive(Serialize, JsonSchema)]
struct UsageBucket {
    start_ms: i64,
    chromium: u32,
    camoufox: u32,
    attached: u32,
    // Bytes moved by runs that ended in the bucket, plus running runs' bytes so far in the current one.
    traffic_bytes: u64,
}

fn history(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: HistoryInput = parse(input)?;
        let window = input.window.unwrap_or(UsageWindow::Day);
        let (count, bucket_ms) = match window {
            UsageWindow::Day => (DAY_BUCKETS, HOUR_MS),
            UsageWindow::Week => (WEEK_BUCKETS, WEEK_BUCKET_HOURS * HOUR_MS),
        };

        let now = now_ms();
        let end = (now.div_euclid(bucket_ms) + 1) * bucket_ms;
        let start = end - count * bucket_ms;
        let (spans, ends) = ctx
            .app
            .store
            .run(move |c| {
                Ok((
                    instance::spans(c, start, end)?,
                    instance::traffic_ends(c, start, end)?,
                ))
            })
            .await?;
        let live_bytes: u64 = running_traffic(&ctx.app)
            .iter()
            .map(|run| run.sent_bytes.saturating_add(run.received_bytes))
            .sum();

        let buckets = (0..count)
            .map(|index| {
                let from = start + index * bucket_ms;
                let to = from + bucket_ms;
                let (mut chromium, mut camoufox, mut attached) =
                    (Vec::new(), Vec::new(), Vec::new());
                for span in &spans {
                    let begin = span.started_at.max(from);
                    let finish = span.ended_at.unwrap_or(now).min(to);
                    // Also drops runs that ended the moment they started.
                    if begin >= finish {
                        continue;
                    }

                    let edges = match span.engine.as_str() {
                        ATTACHED => &mut attached,
                        CAMOUFOX => &mut camoufox,
                        _ => &mut chromium,
                    };
                    edges.extend([(begin, 1), (finish, -1)]);
                }

                UsageBucket {
                    start_ms: from,
                    chromium: peak(chromium),
                    camoufox: peak(camoufox),
                    attached: peak(attached),
                    traffic_bytes: ends
                        .iter()
                        .filter(|(ended_at, _)| (from..to).contains(ended_at))
                        .map(|(_, bytes)| *bytes)
                        .chain((from..to).contains(&now).then_some(live_bytes))
                        .sum(),
                }
            })
            .collect();

        output(&HistoryOutput {
            window,
            bucket_ms,
            buckets,
        })
    }
    .boxed()
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TrafficBy {
    Profile,
    Proxy,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TrafficInput {
    group: TrafficBy,
    days: Option<u32>,
}

#[derive(Serialize, JsonSchema)]
struct TrafficOutput {
    group: TrafficBy,
    days: u32,
    items: Vec<TrafficItem>,
}

#[derive(Serialize, JsonSchema)]
struct TrafficItem {
    id: String,
    name: String,
    sent_bytes: u64,
    received_bytes: u64,
    runs: u64,
}

struct RunningTraffic {
    profile: Option<(String, Option<String>)>,
    proxy_id: Option<brr_core::ProxyId>,
    sent_bytes: u64,
    received_bytes: u64,
}

// Open runs that have a forwarder, with what it has counted so far.
fn running_traffic(app: &Arc<App>) -> Vec<RunningTraffic> {
    app.instances
        .views()
        .into_iter()
        .filter(|view| !view.instance.state.is_final())
        .filter_map(|view| {
            let counted = proxies::live_traffic(app, &view.instance.id)?;

            Some(RunningTraffic {
                profile: view
                    .instance
                    .profile_id
                    .map(|id| (id.to_string(), view.instance.profile_name.clone())),
                proxy_id: proxies::instance_proxy_id(app, &view.instance.id),
                sent_bytes: counted.sent_bytes,
                received_bytes: counted.received_bytes,
            })
        })
        .collect()
}

fn traffic(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: TrafficInput = parse(input)?;
        let days = input.days.unwrap_or(DEFAULT_TRAFFIC_DAYS);
        if !(1..=MAX_TRAFFIC_DAYS).contains(&days) {
            return Err(invalid("days", &format!("must be 1 to {MAX_TRAFFIC_DAYS}")));
        }

        let from = now_ms() - i64::from(days) * DAY_MS;
        let group = match input.group {
            TrafficBy::Profile => TrafficGroup::Profile,
            TrafficBy::Proxy => TrafficGroup::Proxy,
        };
        let ended = ctx
            .app
            .store
            .run(move |c| instance::traffic_totals(c, group, from))
            .await?;

        let mut totals: HashMap<String, TrafficTotal> =
            ended.into_iter().map(|row| (row.id.clone(), row)).collect();
        for run in running_traffic(&ctx.app) {
            let (id, name) = match input.group {
                TrafficBy::Profile => match run.profile {
                    Some((id, name)) => (id.clone(), name.unwrap_or(id)),
                    None => continue,
                },
                TrafficBy::Proxy => match run.proxy_id {
                    Some(proxy_id) => (proxy_id.to_string(), proxy_name(&ctx.app, proxy_id).await?),
                    None => continue,
                },
            };
            let entry = totals.entry(id.clone()).or_insert(TrafficTotal {
                id,
                name,
                sent_bytes: 0,
                received_bytes: 0,
                runs: 0,
            });
            entry.sent_bytes = entry.sent_bytes.saturating_add(run.sent_bytes);
            entry.received_bytes = entry.received_bytes.saturating_add(run.received_bytes);
            entry.runs += 1;
        }

        let mut items: Vec<TrafficItem> = totals
            .into_values()
            .map(|row| TrafficItem {
                id: row.id,
                name: row.name,
                sent_bytes: row.sent_bytes,
                received_bytes: row.received_bytes,
                runs: row.runs,
            })
            .collect();
        items.sort_by(|a, b| {
            (b.sent_bytes.saturating_add(b.received_bytes))
                .cmp(&a.sent_bytes.saturating_add(a.received_bytes))
                .then_with(|| a.id.cmp(&b.id))
        });

        output(&TrafficOutput {
            group: input.group,
            days,
            items,
        })
    }
    .boxed()
}

// The Proxy table has no names, so a Proxy is shown by its endpoint.
async fn proxy_name(app: &Arc<App>, proxy_id: brr_core::ProxyId) -> Result<String, Error> {
    let found = app.store.run(move |c| proxy::get(c, &proxy_id)).await;

    match found {
        Ok(found) => Ok(format!("{}:{}", found.host, found.port)),
        Err(Error::NotFound { .. }) => Ok(proxy_id.to_string()),
        Err(error) => Err(error),
    }
}

// Ends sort before starts at the same instant, so back-to-back runs never count as overlapping.
fn peak(mut edges: Vec<(i64, i32)>) -> u32 {
    edges.sort_unstable();

    let mut running = 0;
    let mut peak = 0;
    for (_, step) in edges {
        running += step;
        peak = peak.max(running);
    }

    u32::try_from(peak).unwrap_or_default()
}
