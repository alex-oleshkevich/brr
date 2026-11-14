use brr_core::{
    CookieFormat, Error, Http, InstanceId, Method, OperationSpec, TextOut, TextUpload, Toolset,
    schema,
};
use brr_store::profile;
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, output, parse};
use crate::{
    app::Ctx,
    services::cookies::{self, Target},
};

static EXPORT: OperationSpec = OperationSpec {
    name: "profile.cookies_export",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Export a Profile's cookies as JSON storageState or Netscape cookies.txt, from a running Instance or the stopped Profile's files",
    http: Http {
        method: Method::Get,
        path: "/v1/profiles/{profile}/cookies",
    },
    cli: &["profile", "cookies", "export"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_cookies_export"),
    read_only: true,
    capability: None,
    input_schema: schema::<ExportInput>,
    output_schema: schema::<ExportOutput>,
};

static IMPORT: OperationSpec = OperationSpec {
    name: "profile.cookies_import",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Import cookies into a Profile, merging with the ones it has; into a running Instance or the stopped Profile's files",
    http: Http {
        method: Method::Put,
        path: "/v1/profiles/{profile}/cookies",
    },
    cli: &["profile", "cookies", "import"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_cookies_import"),
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
struct ExportInput {
    profile: String,
    format: Option<CookieFormat>,
    instance_id: Option<InstanceId>,
}

#[derive(Serialize, JsonSchema)]
struct ExportOutput {
    format: CookieFormat,
    count: u32,
    data: TextOut,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportInput {
    profile: String,
    format: Option<CookieFormat>,
    data: TextUpload,
    instance_id: Option<InstanceId>,
}

#[derive(Serialize, JsonSchema)]
struct ImportOutput {
    imported: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

pub(super) async fn find(ctx: &Ctx, name: String) -> Result<brr_core::Profile, Error> {
    ctx.app.store.run(move |c| profile::find(c, &name)).await
}

fn export(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ExportInput = parse(input)?;
        let format = input.format.unwrap_or(CookieFormat::Json);
        let found = find(&ctx, input.profile).await?;

        // Reads never take a Lease (LC4), so there is nothing to guard.
        let exported =
            cookies::export(&ctx.app, &found, format, input.instance_id.as_ref()).await?;

        output(&ExportOutput {
            format,
            count: exported.count,
            data: TextOut(exported.data),
        })
    }
    .boxed()
}

fn import(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ImportInput = parse(input)?;
        let format = input.format.unwrap_or(CookieFormat::Json);
        let found = find(&ctx, input.profile).await?;
        let parsed = cookies::parse(format, &input.data.0)?;

        let target = cookies::select(&ctx.app, &found, input.instance_id.as_ref()).await?;
        if let Target::Instance(id) = &target {
            ctx.app
                .leases
                .guard(&ctx.app, &ctx.client, id, None, IMPORT.read_only)
                .await?;
        }
        let applied = cookies::apply(&ctx.app, &found, parsed, &target).await?;
        let note = (applied.converted > 0).then(|| {
            format!(
                "{} session cookies were stored as cookies that expire in 400 days: a stopped Camoufox profile keeps no session cookies",
                applied.converted
            )
        });

        output(&ImportOutput {
            imported: applied.imported,
            note,
        })
    }
    .boxed()
}
