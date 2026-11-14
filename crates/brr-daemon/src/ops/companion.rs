use brr_core::{Error, FileOut, Http, Method, OperationSpec, Toolset, schema};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, output, parse};
use crate::{app::Ctx, services::relay::companion};

static COMPANION: OperationSpec = OperationSpec {
    name: "extension.companion",
    group: "extension",
    toolset: Toolset::Profiles,
    summary: "Download the brr companion extension as a zip, ready to load into your own Chrome",
    http: Http {
        method: Method::Get,
        path: "/v1/extensions/companion",
    },
    cli: &["extension", "companion"],
    cli_positional: &[],
    mcp_tool: None,
    read_only: true,
    capability: None,
    input_schema: schema::<CompanionInput>,
    output_schema: schema::<CompanionOutput>,
};

inventory::submit!(OpRegistration {
    spec: &COMPANION,
    handler: companion
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CompanionInput {}

#[derive(Serialize, JsonSchema)]
struct CompanionOutput {
    file: FileOut,
    filename: &'static str,
}

fn companion(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let _: CompanionInput = parse(input)?;

        output(&CompanionOutput {
            file: FileOut(companion::zip(&ctx.app)?.into()),
            filename: companion::FILENAME,
        })
    }
    .boxed()
}
