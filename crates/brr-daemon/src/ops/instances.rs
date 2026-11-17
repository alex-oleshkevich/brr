use std::{collections::HashMap, sync::Arc, time::Duration};

use brr_core::{
    AdmissionMode, BuildRef, BuildSource, Capability, ConflictReason, Display, EndReason, Error,
    HostId, Http, Instance, InstanceId, InstanceKind, InstanceState, InstanceTrigger, Lease,
    Method, OperationSpec, Page, ProfileId, ReleaseReason, SandboxMode, Tab, TabState, Toolset,
    WaitUntil, schema,
};
use brr_store::{
    build,
    instance::{self, Filter},
    profile, template,
};
use futures::{
    FutureExt,
    future::{BoxFuture, join_all},
};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;

use super::{
    MAX_PRIORITY, OpRegistration, invalid, output, page_url, parse,
    tabs::{DEFAULT_NAVIGATE_TIMEOUT_MS, tab_view},
};
use crate::{
    app::{App, Ctx},
    internal,
    services::{
        host,
        instances::{LaunchRequest, promote},
        profiles::validate_name,
        proxies,
    },
};

const EPOCH_MS_HINT: &str = "expects epoch milliseconds, e.g. 1794864186771";
const INSTANCE_ID_HINT: &str = "not a valid InstanceId; expected a 26-character ULID";

static LAUNCH: OperationSpec = OperationSpec {
    name: "instance.launch",
    group: "instance",
    toolset: Toolset::Instances,
    summary: "Launch a Profile; returns the Instance, its first Tab and your Lease on it",
    http: Http {
        method: Method::Post,
        path: "/v1/profiles/{profile}/launch",
    },
    cli: &["profile", "launch"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_launch"),
    read_only: false,
    capability: None,
    input_schema: schema::<LaunchInput>,
    output_schema: schema::<LaunchOutput>,
};

static LIST: OperationSpec = OperationSpec {
    name: "instance.list",
    group: "instance",
    toolset: Toolset::Instances,
    summary: "List Instances, newest first, optionally only some states (comma-separated), one Profile or Template, one end reason or a time window",
    http: Http {
        method: Method::Get,
        path: "/v1/instances",
    },
    cli: &["instance", "list"],
    cli_positional: &[],
    mcp_tool: Some("instance_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ListOutput>,
};

static GET: OperationSpec = OperationSpec {
    name: "instance.get",
    group: "instance",
    toolset: Toolset::Instances,
    summary: "Show one Instance",
    http: Http {
        method: Method::Get,
        path: "/v1/instances/{instance_id}",
    },
    cli: &["instance", "get"],
    cli_positional: &["instance_id"],
    mcp_tool: Some("instance_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<InstanceInput>,
    output_schema: schema::<Instance>,
};

static STOP: OperationSpec = OperationSpec {
    name: "instance.stop",
    group: "instance",
    toolset: Toolset::Instances,
    summary: "Stop an Instance and close its Tabs; another Client's Lease stops it unless `force` is set, which ends that Client's calls with LeaseLost",
    http: Http {
        method: Method::Post,
        path: "/v1/instances/{instance_id}/stop",
    },
    cli: &["instance", "stop"],
    cli_positional: &["instance_id"],
    mcp_tool: Some("instance_stop"),
    read_only: false,
    capability: None,
    input_schema: schema::<StopInput>,
    output_schema: schema::<Instance>,
};

static PROMOTE: OperationSpec = OperationSpec {
    name: "instance.promote",
    group: "instance",
    toolset: Toolset::Instances,
    summary: "Keep an ephemeral Instance's data: when it stops, its folder becomes a new Persistent Profile",
    http: Http {
        method: Method::Post,
        path: "/v1/instances/{instance_id}/promote",
    },
    cli: &["instance", "promote"],
    cli_positional: &["instance_id"],
    mcp_tool: Some("instance_promote"),
    read_only: false,
    capability: None,
    input_schema: schema::<PromoteInput>,
    output_schema: schema::<Instance>,
};

inventory::submit!(OpRegistration {
    spec: &LAUNCH,
    handler: launch
});
inventory::submit!(OpRegistration {
    spec: &LIST,
    handler: list
});
inventory::submit!(OpRegistration {
    spec: &GET,
    handler: get
});
inventory::submit!(OpRegistration {
    spec: &STOP,
    handler: stop
});
inventory::submit!(OpRegistration {
    spec: &PROMOTE,
    handler: promote
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LaunchInput {
    profile: String,
    priority: Option<u8>,
    admission: Option<AdmissionMode>,
    exclusive: Option<bool>,
    display: Option<Display>,
    force: Option<bool>,
    url: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct LaunchOutput {
    instance: Instance,
    tab: Tab,
    lease: Lease,
    instance_lease: Option<Lease>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    state: Option<String>,
    profile: Option<String>,
    template: Option<String>,
    end_reason: Option<EndReason>,
    #[serde(default, deserialize_with = "epoch_ms")]
    #[schemars(description = "Epoch milliseconds, e.g. 1794864186771")]
    since: Option<i64>,
    #[serde(default, deserialize_with = "epoch_ms")]
    #[schemars(description = "Epoch milliseconds, e.g. 1794864186771")]
    until: Option<i64>,
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    instances: Vec<Instance>,
    next_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InstanceInput {
    // Text, so a Profile name passed by mistake gets a hint instead of a bare ULID error.
    #[schemars(with = "InstanceId")]
    instance_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StopInput {
    instance_id: InstanceId,
    force: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PromoteInput {
    instance_id: InstanceId,
    name: String,
}

// Names and counts that the row does not carry; live state wins over the row while an owner runs.
// Builds, Profiles and Leases are read once for the whole batch.
async fn instance_views(app: &Arc<App>, stored: Vec<Instance>) -> Result<Vec<Instance>, Error> {
    let mut profile_ids: Vec<ProfileId> = stored.iter().filter_map(|i| i.profile_id).collect();
    profile_ids.sort();
    profile_ids.dedup();
    let (builds, profiles, docker_urls) = app
        .store
        .run(move |c| {
            let mut profiles = HashMap::new();
            for id in profile_ids {
                if let Some(found) = absent_ok(profile::get(c, &id))? {
                    profiles.insert(id, found.name);
                }
            }
            let docker_urls: HashMap<HostId, String> = brr_store::host::all(c)?
                .into_iter()
                .map(|found| (found.id, found.docker_url))
                .collect();

            Ok((build::all(c)?, profiles, docker_urls))
        })
        .await?;
    let mut lease_counts: HashMap<InstanceId, u32> = HashMap::new();
    for lease in app.leases.list(None, None).await {
        *lease_counts.entry(lease.instance_id).or_default() += 1;
    }

    // A container's processes are not in the daemon's pid tree; Docker counts them.
    let in_docker = |instance: &Instance| instance.sandbox == Some(SandboxMode::Docker);
    let mut rss = if stored.iter().any(|i| counts_memory(i) && !in_docker(i)) {
        let browsers: Vec<_> = stored
            .iter()
            .filter(|instance| counts_memory(instance) && !in_docker(instance))
            .filter_map(|instance| Some((instance.id, instance.pid?)))
            .collect();

        host::pss_by_instance(browsers).await
    } else {
        HashMap::new()
    };
    // Each Docker host (None: brr's own) runs its own `docker stats`, all at once.
    let mut docker_hosts: Vec<Option<&str>> = stored
        .iter()
        .filter(|i| counts_memory(i) && in_docker(i))
        .filter_map(|i| match i.host_id {
            None => Some(None),
            Some(host_id) => docker_urls.get(&host_id).map(|url| Some(url.as_str())),
        })
        .collect();
    docker_hosts.sort_unstable();
    docker_hosts.dedup();
    let read = docker_hosts.into_iter().map(|url| app.sandbox.rss(url));
    for found in join_all(read).await {
        rss.extend(found);
    }

    let views = stored
        .into_iter()
        .map(|row| {
            let stored_traffic = row.traffic.clone();
            let (mut instance, tab_count) = match app.instances.view(&row.id) {
                Some(view) => {
                    let open = view.tabs.iter().filter(|t| t.state != TabState::Closed);

                    (
                        view.instance,
                        u32::try_from(open.count()).unwrap_or(u32::MAX),
                    )
                }
                None => (row, 0),
            };

            let found = instance
                .build_id
                .and_then(|id| builds.iter().find(|b| b.id == id));
            // The stored name survives the profile's deletion; a live profile's current name wins.
            if let Some(name) = instance.profile_id.and_then(|id| profiles.get(&id)) {
                instance.profile_name = Some(name.clone());
            }
            instance.capabilities = match (found, instance.kind) {
                (Some(build), _) => build.capabilities.clone(),
                (None, InstanceKind::Live) => live_capabilities(),
                (None, InstanceKind::Managed) => Vec::new(),
            };
            instance.build = found.map(|b| BuildRef {
                build_id: b.id,
                source: b.source,
                version: b.version.clone(),
                label: format!("{} {}", b.name, b.version),
            });
            instance.tab_count = tab_count;
            // The meter's live count while the run has a forwarder, else what the record kept.
            instance.traffic = proxies::live_traffic(app, &instance.id).or(stored_traffic);
            instance.lease_count = lease_counts.get(&instance.id).copied().unwrap_or(0);
            instance.rss_bytes = counts_memory(&instance)
                .then(|| rss.get(&instance.id).copied())
                .flatten();

            instance
        })
        .collect();

    Ok(views)
}

// A Live browser without a detected system Build is some Chromium-family browser; brr loaded
// nothing into it.
fn live_capabilities() -> Vec<Capability> {
    brr_builds::capabilities(BuildSource::System)
        .into_iter()
        .filter(|capability| *capability != Capability::UnpackedExtensions)
        .collect()
}

fn counts_memory(instance: &Instance) -> bool {
    instance.kind == InstanceKind::Managed && !instance.state.is_final()
}

pub(super) async fn instance_view(app: &Arc<App>, stored: Instance) -> Result<Instance, Error> {
    instance_views(app, vec![stored])
        .await?
        .pop()
        .ok_or_else(|| internal("instance view lost its instance"))
}

// A Profile or Build may be deleted while its final Instances stay in the history.
fn absent_ok<T>(found: Result<T, Error>) -> Result<Option<T>, Error> {
    match found {
        Ok(value) => Ok(Some(value)),
        Err(Error::NotFound { .. }) => Ok(None),
        Err(error) => Err(error),
    }
}

// serde's unknown-variant message already names every valid state.
fn parse_states(list: &str) -> Result<Vec<InstanceState>, Error> {
    list.split(',')
        .map(|name| {
            serde_json::from_value(Value::String(name.trim().to_string())).map_err(|e| {
                invalid(
                    "state",
                    &format!("{e}; separate several states with commas"),
                )
            })
        })
        .collect()
}

fn epoch_ms<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
    Option::<i64>::deserialize(deserializer).map_err(|_| de::Error::custom(EPOCH_MS_HINT))
}

fn launch(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: LaunchInput = parse(input)?;
        input.url.as_deref().map(page_url).transpose()?;
        let defaults = ctx.app.settings().admission.clone();
        let priority = input.priority.unwrap_or(defaults.priority);
        if priority > MAX_PRIORITY {
            return Err(invalid("priority", &format!("must be between 0 and {MAX_PRIORITY}")));
        }

        let profile_ref = input.profile;
        let profile = ctx
            .app
            .store
            .run(move |c| profile::find(c, &profile_ref))
            .await?;
        let request = LaunchRequest {
            profile,
            display: input.display,
            priority,
            admission: input.admission.unwrap_or(defaults.default),
            force: input.force.unwrap_or(false),
        };

        let (launched, first_tab) = ctx.app.instances.launch(&ctx.app, request).await?;
        let exclusive = input.exclusive.unwrap_or(false);
        let (lease, instance_lease) =
            match settle_launch(&ctx, &launched, &first_tab, exclusive, input.url).await {
                Ok(leases) => leases,
                Err(error) => {
                    let stopped = ctx
                        .app
                        .instances
                        .stop(&ctx.app, &launched.id, EndReason::LaunchError)
                        .await;
                    if let Err(stop_error) = stopped {
                        tracing::warn!(instance_id = %launched.id, %stop_error, "half-launched instance not stopped");
                    }

                    return Err(error);
                }
            };

        output(&LaunchOutput {
            instance: instance_view(&ctx.app, launched).await?,
            tab: tab_view(&ctx.app, first_tab).await,
            lease,
            instance_lease,
        })
    }
    .boxed()
}

async fn settle_launch(
    ctx: &Ctx,
    launched: &Instance,
    first_tab: &Tab,
    exclusive: bool,
    url: Option<String>,
) -> Result<(Lease, Option<Lease>), Error> {
    let lease = ctx
        .app
        .leases
        .acquire_tab(&ctx.app, &ctx.client, &launched.id, &first_tab.id)
        .await?;
    // The Instance is new, so nobody else can hold it and this never waits.
    let instance_lease = if exclusive {
        Some(
            ctx.app
                .leases
                .acquire_exclusive(&ctx.app, &ctx.client, &launched.id)
                .await?,
        )
    } else {
        None
    };

    if let Some(url) = url {
        ctx.app
            .instances
            .navigate(
                &launched.id,
                &first_tab.id,
                url,
                WaitUntil::Load,
                Duration::from_millis(DEFAULT_NAVIGATE_TIMEOUT_MS),
            )
            .await?;
    }
    ctx.app
        .clients
        .set_current_tab(&ctx.app, &ctx.client.client_id, Some(&first_tab.id))
        .await?;

    Ok((lease, instance_lease))
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let states = input
            .state
            .as_deref()
            .map(parse_states)
            .transpose()?
            .unwrap_or_default();
        let page = Page::new(input.limit, input.cursor);

        let (stored, next_cursor) = ctx
            .app
            .store
            .run(move |c| {
                let profile_id: Option<ProfileId> = match input.profile {
                    Some(profile_ref) => Some(profile::find(c, &profile_ref)?.id),
                    None => None,
                };
                // A deleted Template still names its past runs, so only a name nothing knows fails.
                if let Some(template_ref) = &input.template
                    && absent_ok(template::find(c, template_ref))?.is_none()
                    && !instance::names_template(c, template_ref)?
                {
                    return Err(Error::NotFound {
                        what: format!("template {template_ref}"),
                    });
                }

                let filter = Filter {
                    states,
                    profile_id,
                    template: input.template,
                    end_reason: input.end_reason,
                    since: input.since,
                    until: input.until,
                };

                instance::list(c, &filter, &page)
            })
            .await?;

        let instances = instance_views(&ctx.app, stored).await?;

        output(&ListOutput {
            instances,
            next_cursor,
        })
    }
    .boxed()
}

fn get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: InstanceInput = parse(input)?;
        let raw = input.instance_id;
        let stored = ctx
            .app
            .store
            .run(move |c| match raw.parse::<InstanceId>() {
                Ok(instance_id) => instance::get(c, &instance_id),
                Err(_) if absent_ok(profile::find(c, &raw))?.is_some() => Err(invalid(
                    "instance_id",
                    &format!(
                        "{raw:?} is a Profile, not an Instance id; run `brr instance list --profile {raw}` to find its Instances"
                    ),
                )),
                Err(_) => Err(invalid("instance_id", INSTANCE_ID_HINT)),
            })
            .await?;

        output(&instance_view(&ctx.app, stored).await?)
    }
    .boxed()
}

fn stop(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: StopInput = parse(input)?;
        let instance_id = input.instance_id;

        // The state decides first: an Instance that cannot stop answers InvalidTransition, leased or not.
        let state = ctx
            .app
            .store
            .run(move |c| instance::get(c, &instance_id))
            .await?
            .state;
        if state != InstanceState::Queued {
            state.transition(InstanceTrigger::Stop)?;
        }

        if input.force.unwrap_or(false) {
            tracing::info!(%instance_id, client_id = %ctx.client.client_id, "instance force-stopped");
            ctx.app
                .leases
                .release_instance(&ctx.app, &instance_id, ReleaseReason::Released)
                .await?;
        } else {
            ctx.app
                .leases
                .guard_stop(&ctx.client, &input.instance_id)
                .await?;
        }
        let stopped = ctx
            .app
            .instances
            .stop(&ctx.app, &input.instance_id, EndReason::Stop)
            .await?;

        output(&instance_view(&ctx.app, stopped).await?)
    }
    .boxed()
}

fn promote(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: PromoteInput = parse(input)?;
        validate_name(&input.name, "name")?;

        ctx.app
            .leases
            .guard(
                &ctx.app,
                &ctx.client,
                &input.instance_id,
                None,
                PROMOTE.read_only,
            )
            .await?;

        let instance_id = input.instance_id;
        let name = input.name.clone();
        let stored = ctx
            .app
            .store
            .run(move |c| {
                match profile::find(c, &name) {
                    Ok(_) => {
                        return Err(Error::Conflict {
                            reason: ConflictReason::NameTaken,
                            detail: None,
                        });
                    }
                    Err(Error::NotFound { .. }) => {}
                    Err(error) => return Err(error),
                }

                instance::get(c, &instance_id)
            })
            .await?;
        promote::check_promotable(&stored)?;

        let promoted = ctx
            .app
            .instances
            .promote(&input.instance_id, input.name)
            .await?;

        output(&instance_view(&ctx.app, promoted).await?)
    }
    .boxed()
}
