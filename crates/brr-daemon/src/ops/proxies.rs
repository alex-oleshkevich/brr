use brr_core::{
    ConflictReason, Error, Http, Method, OperationSpec, Page, Pool, PoolId, PoolMode, Proxy,
    ProxyId, ProxyScheme, Secret, TextUpload, Toolset, schema,
};
use brr_store::{pool, profile, proxy};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, output, parse};
use crate::{
    app::Ctx,
    services::{
        profiles::validate_name,
        proxies::{self, NewProxy, ProxyChanges},
    },
};

static PROXY_LIST: OperationSpec = OperationSpec {
    name: "proxy.list",
    group: "proxy",
    toolset: Toolset::Network,
    summary: "List proxies with their last health check; passwords show only whether they are set",
    http: Http {
        method: Method::Get,
        path: "/v1/proxies",
    },
    cli: &["proxy", "list"],
    cli_positional: &[],
    mcp_tool: Some("proxy_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ProxyListOutput>,
};

static PROXY_CREATE: OperationSpec = OperationSpec {
    name: "proxy.create",
    group: "proxy",
    toolset: Toolset::Network,
    summary: "Add a proxy; it is health-checked at once",
    http: Http {
        method: Method::Post,
        path: "/v1/proxies",
    },
    cli: &["proxy", "create"],
    cli_positional: &[],
    mcp_tool: Some("proxy_create"),
    read_only: false,
    capability: None,
    input_schema: schema::<CreateInput>,
    output_schema: schema::<Proxy>,
};

static PROXY_UPDATE: OperationSpec = OperationSpec {
    name: "proxy.update",
    group: "proxy",
    toolset: Toolset::Network,
    summary: "Change a proxy; an absent password is kept, and the health check runs again",
    http: Http {
        method: Method::Patch,
        path: "/v1/proxies/{proxy_id}",
    },
    cli: &["proxy", "update"],
    cli_positional: &["proxy_id"],
    mcp_tool: Some("proxy_update"),
    read_only: false,
    capability: None,
    input_schema: schema::<UpdateInput>,
    output_schema: schema::<Proxy>,
};

static PROXY_DELETE: OperationSpec = OperationSpec {
    name: "proxy.delete",
    group: "proxy",
    toolset: Toolset::Network,
    summary: "Delete a proxy that no Profile uses",
    http: Http {
        method: Method::Delete,
        path: "/v1/proxies/{proxy_id}",
    },
    cli: &["proxy", "delete"],
    cli_positional: &["proxy_id"],
    mcp_tool: Some("proxy_delete"),
    read_only: false,
    capability: None,
    input_schema: schema::<ProxyIdInput>,
    output_schema: schema::<Proxy>,
};

static PROXY_IMPORT: OperationSpec = OperationSpec {
    name: "proxy.import",
    group: "proxy",
    toolset: Toolset::Network,
    summary: "Add many proxies from lines of host:port, host:port:user:pass or scheme://user:pass@host:port (IPv6 addresses cannot be imported; add them one at a time)",
    http: Http {
        method: Method::Post,
        path: "/v1/proxies/import",
    },
    cli: &["proxy", "import"],
    cli_positional: &[],
    mcp_tool: Some("proxy_import"),
    read_only: false,
    capability: None,
    input_schema: schema::<ImportInput>,
    output_schema: schema::<ImportOutput>,
};

static PROXY_CHECK: OperationSpec = OperationSpec {
    name: "proxy.check",
    group: "proxy",
    toolset: Toolset::Network,
    summary: "Check a proxy now and return its health",
    http: Http {
        method: Method::Post,
        path: "/v1/proxies/{proxy_id}/check",
    },
    cli: &["proxy", "check"],
    cli_positional: &["proxy_id"],
    mcp_tool: Some("proxy_check"),
    read_only: false,
    capability: None,
    input_schema: schema::<ProxyIdInput>,
    output_schema: schema::<Proxy>,
};

static POOL_LIST: OperationSpec = OperationSpec {
    name: "pool.list",
    group: "pool",
    toolset: Toolset::Network,
    summary: "List proxy pools",
    http: Http {
        method: Method::Get,
        path: "/v1/proxy-pools",
    },
    cli: &["pool", "list"],
    cli_positional: &[],
    mcp_tool: Some("pool_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<PoolListOutput>,
};

static POOL_CREATE: OperationSpec = OperationSpec {
    name: "pool.create",
    group: "pool",
    toolset: Toolset::Network,
    summary: "Create a pool of proxies: sticky per Profile or rotated per launch",
    http: Http {
        method: Method::Post,
        path: "/v1/proxy-pools",
    },
    cli: &["pool", "create"],
    cli_positional: &["name"],
    mcp_tool: Some("pool_create"),
    read_only: false,
    capability: None,
    input_schema: schema::<PoolCreateInput>,
    output_schema: schema::<Pool>,
};

static POOL_UPDATE: OperationSpec = OperationSpec {
    name: "pool.update",
    group: "pool",
    toolset: Toolset::Network,
    summary: "Rename a pool or change its mode or members",
    http: Http {
        method: Method::Patch,
        path: "/v1/proxy-pools/{pool}",
    },
    cli: &["pool", "update"],
    cli_positional: &["pool"],
    mcp_tool: Some("pool_update"),
    read_only: false,
    capability: None,
    input_schema: schema::<PoolUpdateInput>,
    output_schema: schema::<Pool>,
};

static POOL_DELETE: OperationSpec = OperationSpec {
    name: "pool.delete",
    group: "pool",
    toolset: Toolset::Network,
    summary: "Delete a pool that no Profile uses",
    http: Http {
        method: Method::Delete,
        path: "/v1/proxy-pools/{pool}",
    },
    cli: &["pool", "delete"],
    cli_positional: &["pool"],
    mcp_tool: Some("pool_delete"),
    read_only: false,
    capability: None,
    input_schema: schema::<PoolInput>,
    output_schema: schema::<Pool>,
};

inventory::submit!(OpRegistration {
    spec: &PROXY_LIST,
    handler: proxy_list
});
inventory::submit!(OpRegistration {
    spec: &PROXY_CREATE,
    handler: proxy_create
});
inventory::submit!(OpRegistration {
    spec: &PROXY_UPDATE,
    handler: proxy_update
});
inventory::submit!(OpRegistration {
    spec: &PROXY_DELETE,
    handler: proxy_delete
});
inventory::submit!(OpRegistration {
    spec: &PROXY_IMPORT,
    handler: proxy_import
});
inventory::submit!(OpRegistration {
    spec: &PROXY_CHECK,
    handler: proxy_check
});
inventory::submit!(OpRegistration {
    spec: &POOL_LIST,
    handler: pool_list
});
inventory::submit!(OpRegistration {
    spec: &POOL_CREATE,
    handler: pool_create
});
inventory::submit!(OpRegistration {
    spec: &POOL_UPDATE,
    handler: pool_update
});
inventory::submit!(OpRegistration {
    spec: &POOL_DELETE,
    handler: pool_delete
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ProxyListOutput {
    proxies: Vec<Proxy>,
    next_cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct PoolListOutput {
    pools: Vec<Pool>,
    next_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    scheme: ProxyScheme,
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<Secret>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateInput {
    proxy_id: ProxyId,
    scheme: Option<ProxyScheme>,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password: Option<Secret>,
    clear_password: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProxyIdInput {
    proxy_id: ProxyId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportInput {
    scheme: ProxyScheme,
    text: TextUpload,
}

#[derive(Serialize, JsonSchema)]
struct ImportOutput {
    imported: u32,
    duplicates: u32,
    invalid_lines: Vec<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PoolCreateInput {
    name: String,
    mode: PoolMode,
    proxy_ids: Vec<ProxyId>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PoolUpdateInput {
    pool: String,
    name: Option<String>,
    mode: Option<PoolMode>,
    proxy_ids: Option<Vec<ProxyId>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PoolInput {
    pool: String,
}

fn proxy_list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let (proxies, next_cursor) =
            proxies::list(&ctx.app, Page::new(input.limit, input.cursor)).await?;

        output(&ProxyListOutput {
            proxies,
            next_cursor,
        })
    }
    .boxed()
}

fn proxy_create(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: CreateInput = parse(input)?;
        let created = proxies::create(
            &ctx.app,
            NewProxy {
                scheme: input.scheme,
                host: input.host,
                port: input.port,
                username: input.username,
                password: input.password,
            },
        )
        .await?;

        output(&created)
    }
    .boxed()
}

fn proxy_update(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: UpdateInput = parse(input)?;
        let updated = proxies::update(
            &ctx.app,
            &input.proxy_id,
            ProxyChanges {
                scheme: input.scheme,
                host: input.host,
                port: input.port,
                username: input.username,
                password: input.password,
                clear_password: input.clear_password.unwrap_or(false),
            },
        )
        .await?;

        output(&updated)
    }
    .boxed()
}

fn proxy_delete(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ProxyIdInput = parse(input)?;

        output(&proxies::delete(&ctx.app, &input.proxy_id).await?)
    }
    .boxed()
}

fn proxy_import(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ImportInput = parse(input)?;
        let imported = proxies::import(&ctx.app, input.scheme, &input.text.0).await?;

        output(&ImportOutput {
            imported: u32::try_from(imported.ids.len()).unwrap_or(u32::MAX),
            duplicates: imported.duplicates,
            invalid_lines: imported.invalid_lines,
        })
    }
    .boxed()
}

fn proxy_check(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ProxyIdInput = parse(input)?;

        output(&proxies::check(&ctx.app, &input.proxy_id).await?)
    }
    .boxed()
}

fn pool_list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let page = Page::new(input.limit, input.cursor);
        let (pools, next_cursor) = ctx.app.store.run(move |c| pool::list(c, &page)).await?;

        output(&PoolListOutput { pools, next_cursor })
    }
    .boxed()
}

fn ensure_proxies(c: &brr_store::Connection, ids: &[ProxyId]) -> Result<(), Error> {
    ids.iter().try_for_each(|id| proxy::get(c, id).map(drop))
}

fn pool_create(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: PoolCreateInput = parse(input)?;
        validate_name(&input.name, "name")?;

        let created = ctx
            .app
            .store
            .run(move |c| {
                ensure_proxies(c, &input.proxy_ids)?;
                let created = Pool {
                    id: PoolId::new(),
                    name: input.name,
                    mode: input.mode,
                    proxy_ids: input.proxy_ids,
                };
                pool::insert(c, &created)?;

                pool::get(c, &created.id)
            })
            .await?;

        output(&created)
    }
    .boxed()
}

fn pool_update(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: PoolUpdateInput = parse(input)?;
        if let Some(name) = &input.name {
            validate_name(name, "name")?;
        }

        let updated = ctx
            .app
            .store
            .run(move |c| {
                let mut found = pool::find(c, &input.pool)?;
                found.name = input.name.unwrap_or(found.name);
                found.mode = input.mode.unwrap_or(found.mode);
                found.proxy_ids = input.proxy_ids.unwrap_or(found.proxy_ids);
                ensure_proxies(c, &found.proxy_ids)?;
                pool::update(c, &found)?;

                pool::get(c, &found.id)
            })
            .await?;

        output(&updated)
    }
    .boxed()
}

fn pool_delete(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: PoolInput = parse(input)?;

        let deleted = ctx
            .app
            .store
            .run(move |c| {
                let found = pool::find(c, &input.pool)?;
                let users = profile::using_pool(c, &found.id)?;
                if !users.is_empty() {
                    let names: Vec<String> = users.into_iter().map(|(_, name)| name).collect();

                    return Err(Error::Conflict {
                        reason: ConflictReason::Referenced,
                        detail: Some(format!("profile {}", names.join(", "))),
                    });
                }
                pool::delete(c, &found.id)?;

                Ok(found)
            })
            .await?;

        output(&deleted)
    }
    .boxed()
}
