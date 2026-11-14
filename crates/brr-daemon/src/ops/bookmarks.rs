use brr_core::{Error, Http, Method, OperationSpec, TextOut, TextUpload, Toolset, schema};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, cookies::find, output, parse};
use crate::{app::Ctx, services::bookmarks};

static EXPORT: OperationSpec = OperationSpec {
    name: "profile.bookmarks_export",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Export a stopped Profile's bookmarks as an HTML file",
    http: Http {
        method: Method::Get,
        path: "/v1/profiles/{profile}/bookmarks",
    },
    cli: &["profile", "bookmarks", "export"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_bookmarks_export"),
    read_only: true,
    capability: None,
    input_schema: schema::<ProfileInput>,
    output_schema: schema::<ExportOutput>,
};

static IMPORT: OperationSpec = OperationSpec {
    name: "profile.bookmarks_import",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Add bookmarks from an HTML file to a stopped Profile's bookmarks bar",
    http: Http {
        method: Method::Put,
        path: "/v1/profiles/{profile}/bookmarks",
    },
    cli: &["profile", "bookmarks", "import"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_bookmarks_import"),
    read_only: false,
    capability: None,
    input_schema: schema::<ImportInput>,
    output_schema: schema::<ImportOutput>,
};

inventory::submit!(OpRegistration {
    spec: &EXPORT,
    handler: export
});
inventory::submit!(OpRegistration {
    spec: &IMPORT,
    handler: import
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProfileInput {
    profile: String,
}

#[derive(Serialize, JsonSchema)]
struct ExportOutput {
    count: u32,
    data: TextOut,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportInput {
    profile: String,
    data: TextUpload,
}

#[derive(Serialize, JsonSchema)]
struct ImportOutput {
    imported: u32,
}

fn export(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ProfileInput = parse(input)?;
        let found = find(&ctx, input.profile).await?;
        let (html, count) = bookmarks::export(&ctx.app, &found).await?;

        output(&ExportOutput {
            count,
            data: TextOut(html),
        })
    }
    .boxed()
}

fn import(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ImportInput = parse(input)?;
        let found = find(&ctx, input.profile).await?;
        let imported = bookmarks::import(&ctx.app, &found, &input.data.0).await?;

        output(&ImportOutput { imported })
    }
    .boxed()
}
