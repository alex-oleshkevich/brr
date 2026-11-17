use brr_core::{Empty, Error, Host, Http, Method, OperationSpec, Toolset, schema};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, output, parse};
use crate::{
    app::Ctx,
    services::hosts::{self, HostChanges, NewHost},
};

static HOST_LIST: OperationSpec = OperationSpec {
    name: "host.list",
    group: "host",
    toolset: Toolset::System,
    summary: "List the machines that run browsers in Docker over SSH, with their last check",
    http: Http {
        method: Method::Get,
        path: "/v1/hosts",
    },
    cli: &["host", "list"],
    cli_positional: &[],
    mcp_tool: Some("host_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<ListOutput>,
};

static HOST_GET: OperationSpec = OperationSpec {
    name: "host.get",
    group: "host",
    toolset: Toolset::System,
    summary: "Show one Host by id or name, with its last check",
    http: Http {
        method: Method::Get,
        path: "/v1/hosts/{host}",
    },
    cli: &["host", "get"],
    cli_positional: &["host"],
    mcp_tool: Some("host_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<HostInput>,
    output_schema: schema::<Host>,
};

static HOST_CREATE: OperationSpec = OperationSpec {
    name: "host.create",
    group: "host",
    toolset: Toolset::System,
    summary: "Add a machine that runs browsers through its Docker over SSH (ssh://user@host[:port]); it is checked at once",
    http: Http {
        method: Method::Post,
        path: "/v1/hosts",
    },
    cli: &["host", "create"],
    cli_positional: &["name"],
    mcp_tool: Some("host_create"),
    read_only: false,
    capability: None,
    input_schema: schema::<CreateInput>,
    output_schema: schema::<Host>,
};

static HOST_UPDATE: OperationSpec = OperationSpec {
    name: "host.update",
    group: "host",
    toolset: Toolset::System,
    summary: "Change a Host; it is checked again",
    http: Http {
        method: Method::Patch,
        path: "/v1/hosts/{host}",
    },
    cli: &["host", "update"],
    cli_positional: &["host"],
    mcp_tool: Some("host_update"),
    read_only: false,
    capability: None,
    input_schema: schema::<UpdateInput>,
    output_schema: schema::<Host>,
};

static HOST_DELETE: OperationSpec = OperationSpec {
    name: "host.delete",
    group: "host",
    toolset: Toolset::System,
    summary: "Delete a Host that runs no browsers",
    http: Http {
        method: Method::Delete,
        path: "/v1/hosts/{host}",
    },
    cli: &["host", "delete"],
    cli_positional: &["host"],
    mcp_tool: Some("host_delete"),
    read_only: false,
    capability: None,
    input_schema: schema::<HostInput>,
    output_schema: schema::<Host>,
};

static HOST_CHECK: OperationSpec = OperationSpec {
    name: "host.check",
    group: "host",
    toolset: Toolset::System,
    summary: "Check a Host now: Docker reachable, memory, sandbox image present",
    http: Http {
        method: Method::Post,
        path: "/v1/hosts/{host}/check",
    },
    cli: &["host", "check"],
    cli_positional: &["host"],
    mcp_tool: Some("host_check"),
    read_only: false,
    capability: None,
    input_schema: schema::<HostInput>,
    output_schema: schema::<Host>,
};

inventory::submit!(OpRegistration {
    spec: &HOST_LIST,
    handler: host_list
});
inventory::submit!(OpRegistration {
    spec: &HOST_GET,
    handler: host_get
});
inventory::submit!(OpRegistration {
    spec: &HOST_CREATE,
    handler: host_create
});
inventory::submit!(OpRegistration {
    spec: &HOST_UPDATE,
    handler: host_update
});
inventory::submit!(OpRegistration {
    spec: &HOST_DELETE,
    handler: host_delete
});
inventory::submit!(OpRegistration {
    spec: &HOST_CHECK,
    handler: host_check
});

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    hosts: Vec<Host>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HostInput {
    host: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    name: String,
    docker_url: String,
    max_instances: u32,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateInput {
    host: String,
    name: Option<String>,
    docker_url: Option<String>,
    max_instances: Option<u32>,
    enabled: Option<bool>,
}

fn host_list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        output(&ListOutput {
            hosts: hosts::list(&ctx.app).await?,
        })
    }
    .boxed()
}

fn host_get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: HostInput = parse(input)?;

        output(&hosts::get(&ctx.app, input.host).await?)
    }
    .boxed()
}

fn host_create(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: CreateInput = parse(input)?;
        let created = hosts::create(
            &ctx.app,
            NewHost {
                name: input.name,
                docker_url: input.docker_url,
                max_instances: input.max_instances,
            },
        )
        .await?;

        output(&created)
    }
    .boxed()
}

fn host_update(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: UpdateInput = parse(input)?;
        let updated = hosts::update(
            &ctx.app,
            input.host,
            HostChanges {
                name: input.name,
                docker_url: input.docker_url,
                max_instances: input.max_instances,
                enabled: input.enabled,
            },
        )
        .await?;

        output(&updated)
    }
    .boxed()
}

fn host_delete(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: HostInput = parse(input)?;

        output(&hosts::delete(&ctx.app, input.host).await?)
    }
    .boxed()
}

fn host_check(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: HostInput = parse(input)?;

        output(&hosts::check(&ctx.app, input.host).await?)
    }
    .boxed()
}
