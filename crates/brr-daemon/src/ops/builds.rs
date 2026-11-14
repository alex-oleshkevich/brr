use brr_builds::{Available, Want, available as available_builds};
use brr_core::{
    Build, BuildId, BuildSource, Empty, Error, Http, Method, OperationSpec, Page, Toolset, schema,
};
use brr_store::build;
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, output, parse};
use crate::{
    app::Ctx,
    services::builds::{self, BuildView, ensure_installable, source_urls},
};

const DEFAULT_CHANNEL: &str = "stable";

static LIST: OperationSpec = OperationSpec {
    name: "build.list",
    group: "build",
    toolset: Toolset::Builds,
    summary: "List installed and registered Builds, newest first",
    http: Http {
        method: Method::Get,
        path: "/v1/builds",
    },
    cli: &["build", "list"],
    cli_positional: &[],
    mcp_tool: Some("build_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ListOutput>,
};

static GET: OperationSpec = OperationSpec {
    name: "build.get",
    group: "build",
    toolset: Toolset::Builds,
    summary: "Show one Build, including the progress state of an install",
    http: Http {
        method: Method::Get,
        path: "/v1/builds/{build_id}",
    },
    cli: &["build", "get"],
    cli_positional: &["build_id"],
    mcp_tool: Some("build_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<GetInput>,
    output_schema: schema::<BuildView>,
};

static AVAILABLE: OperationSpec = OperationSpec {
    name: "build.available",
    group: "build",
    toolset: Toolset::Builds,
    summary: "List versions and channels a source offers for install",
    http: Http {
        method: Method::Get,
        path: "/v1/builds/available",
    },
    cli: &["build", "available"],
    cli_positional: &[],
    mcp_tool: Some("build_available"),
    read_only: true,
    capability: None,
    input_schema: schema::<AvailableInput>,
    output_schema: schema::<AvailableOutput>,
};

static INSTALL: OperationSpec = OperationSpec {
    name: "build.install",
    group: "build",
    toolset: Toolset::Builds,
    summary: "Install a Build; returns at once while it downloads, watch build.get or events",
    http: Http {
        method: Method::Post,
        path: "/v1/builds",
    },
    cli: &["build", "install"],
    cli_positional: &[],
    mcp_tool: Some("build_install"),
    read_only: false,
    capability: None,
    input_schema: schema::<InstallInput>,
    output_schema: schema::<Build>,
};

static REMOVE: OperationSpec = OperationSpec {
    name: "build.remove",
    group: "build",
    toolset: Toolset::Builds,
    summary: "Delete an installed Build that no Profile, channel or Instance uses",
    http: Http {
        method: Method::Delete,
        path: "/v1/builds/{build_id}",
    },
    cli: &["build", "remove"],
    cli_positional: &["build_id"],
    mcp_tool: Some("build_remove"),
    read_only: false,
    capability: None,
    input_schema: schema::<GetInput>,
    output_schema: schema::<Build>,
};

static DETECT: OperationSpec = OperationSpec {
    name: "build.detect",
    group: "build",
    toolset: Toolset::Builds,
    summary: "Find Chromium-family browsers already on this machine and register them",
    http: Http {
        method: Method::Post,
        path: "/v1/builds/detect",
    },
    cli: &["build", "detect"],
    cli_positional: &[],
    mcp_tool: Some("build_detect"),
    read_only: false,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<DetectOutput>,
};

inventory::submit!(OpRegistration {
    spec: &LIST,
    handler: list
});
inventory::submit!(OpRegistration {
    spec: &GET,
    handler: get
});
inventory::submit!(OpRegistration {
    spec: &AVAILABLE,
    handler: available
});
inventory::submit!(OpRegistration {
    spec: &INSTALL,
    handler: install
});
inventory::submit!(OpRegistration {
    spec: &REMOVE,
    handler: remove
});
inventory::submit!(OpRegistration {
    spec: &DETECT,
    handler: detect
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    builds: Vec<BuildView>,
    next_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetInput {
    build_id: BuildId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AvailableInput {
    source: Option<BuildSource>,
}

#[derive(Serialize, JsonSchema)]
struct AvailableOutput {
    available: Vec<Available>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InstallInput {
    source: BuildSource,
    version: Option<String>,
    channel: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct DetectOutput {
    builds: Vec<Build>,
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let page = Page::new(input.limit, input.cursor);

        let (builds, next_cursor) = ctx
            .app
            .store
            .run(move |c| {
                let (builds, next_cursor) = build::list(c, &page)?;

                Ok((builds::views(c, builds)?, next_cursor))
            })
            .await?;

        output(&ListOutput {
            builds,
            next_cursor,
        })
    }
    .boxed()
}

fn get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: GetInput = parse(input)?;
        let found = ctx
            .app
            .store
            .run(move |c| builds::view(c, build::get(c, &input.build_id)?))
            .await?;

        output(&found)
    }
    .boxed()
}

fn available(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: AvailableInput = parse(input)?;
        let source = input.source.unwrap_or(BuildSource::Cft);
        ensure_installable(source)?;

        let available = available_builds(&ctx.app.http, &source_urls(&ctx.app), source).await?;

        output(&AvailableOutput { available })
    }
    .boxed()
}

fn install(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: InstallInput = parse(input)?;
        let want = match (input.version, input.channel) {
            (Some(version), None) => Want::Version(version),
            (None, Some(channel)) => Want::Channel(channel),
            (None, None) => Want::Channel(DEFAULT_CHANNEL.to_string()),
            (Some(_), Some(_)) => {
                return Err(Error::InvalidArgument {
                    field: "channel".to_string(),
                    message: "give a version or a channel, not both".to_string(),
                });
            }
        };
        let installing = builds::install(&ctx.app, input.source, want).await?;

        output(&installing)
    }
    .boxed()
}

fn remove(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: GetInput = parse(input)?;
        let removed = builds::remove(&ctx.app, &input.build_id).await?;

        output(&removed)
    }
    .boxed()
}

fn detect(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let _: Empty = parse(input)?;
        let builds = builds::detect(&ctx.app).await?;

        output(&DetectOutput { builds })
    }
    .boxed()
}
