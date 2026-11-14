use brr_core::{
    Artifact, ArtifactId, ArtifactKind, Error, Http, InstanceId, Method, OperationSpec, Page,
    ProfileId, Toolset, capture::ReturnMode, schema,
};
use brr_store::artifact::{self, Filter};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, output, parse};
use crate::{
    app::Ctx,
    services::artifacts::{self, Delivery},
};

const DELETE_REASON: &str = "deleted";

static LIST: OperationSpec = OperationSpec {
    name: "artifact.list",
    group: "artifact",
    toolset: Toolset::Browser,
    summary: "List stored Artifacts (screenshots, PDFs, Downloads, HARs), newest first",
    http: Http {
        method: Method::Get,
        path: "/v1/artifacts",
    },
    cli: &["artifact", "list"],
    cli_positional: &[],
    mcp_tool: Some("artifact_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ListOutput>,
};

static GET: OperationSpec = OperationSpec {
    name: "artifact.get",
    group: "artifact",
    toolset: Toolset::Browser,
    summary: "Show one Artifact's record",
    http: Http {
        method: Method::Get,
        path: "/v1/artifacts/{artifact_id}",
    },
    cli: &["artifact", "get"],
    cli_positional: &["artifact_id"],
    mcp_tool: Some("artifact_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<IdInput>,
    output_schema: schema::<Artifact>,
};

static FETCH: OperationSpec = OperationSpec {
    name: "artifact.fetch",
    group: "artifact",
    toolset: Toolset::Browser,
    summary: "Get an Artifact as a URL, as inline base64, or both",
    http: Http {
        method: Method::Get,
        path: "/v1/artifacts/{artifact_id}/fetch",
    },
    cli: &["artifact", "fetch"],
    cli_positional: &["artifact_id"],
    mcp_tool: Some("artifact_fetch"),
    read_only: true,
    capability: None,
    input_schema: schema::<FetchInput>,
    output_schema: schema::<Delivery>,
};

static DELETE: OperationSpec = OperationSpec {
    name: "artifact.delete",
    group: "artifact",
    toolset: Toolset::Storage,
    summary: "Delete an Artifact and its stored file",
    http: Http {
        method: Method::Delete,
        path: "/v1/artifacts/{artifact_id}",
    },
    cli: &["artifact", "delete"],
    cli_positional: &["artifact_id"],
    mcp_tool: Some("artifact_delete"),
    read_only: false,
    capability: None,
    input_schema: schema::<IdInput>,
    output_schema: schema::<Artifact>,
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
    spec: &FETCH,
    handler: fetch
});
inventory::submit!(OpRegistration {
    spec: &DELETE,
    handler: delete
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    kind: Option<ArtifactKind>,
    profile_id: Option<ProfileId>,
    instance_id: Option<InstanceId>,
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    artifacts: Vec<Artifact>,
    next_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct IdInput {
    artifact_id: ArtifactId,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FetchInput {
    artifact_id: ArtifactId,
    #[serde(rename = "return")]
    return_mode: Option<ReturnMode>,
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let filter = Filter {
            kind: input.kind,
            profile_id: input.profile_id,
            instance_id: input.instance_id,
        };
        let page = Page::new(input.limit, input.cursor);
        let (artifacts, next_cursor) = ctx
            .app
            .store
            .run(move |c| artifact::list(c, &filter, &page))
            .await?;

        output(&ListOutput {
            artifacts,
            next_cursor,
        })
    }
    .boxed()
}

fn get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: IdInput = parse(input)?;
        let found = ctx
            .app
            .store
            .run(move |c| artifact::get(c, &input.artifact_id))
            .await?;

        output(&found)
    }
    .boxed()
}

fn fetch(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: FetchInput = parse(input)?;
        let found = ctx
            .app
            .store
            .run(move |c| artifact::get(c, &input.artifact_id))
            .await?;
        let mode = input.return_mode.unwrap_or(ReturnMode::Url);

        output(&artifacts::fetch(&ctx.app, &found, mode).await?)
    }
    .boxed()
}

fn delete(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: IdInput = parse(input)?;
        let found = ctx
            .app
            .store
            .run(move |c| artifact::get(c, &input.artifact_id))
            .await?;
        artifacts::delete(&ctx.app, &found, DELETE_REASON).await?;

        output(&found)
    }
    .boxed()
}
