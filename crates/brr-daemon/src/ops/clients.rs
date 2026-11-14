use brr_core::{
    Client, ClientEndReason, ClientId, ClientKind, Error, Http, Method, OperationSpec, Toolset,
    schema,
};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::{OpRegistration, output, parse};
use crate::app::Ctx;

static CREATE: OperationSpec = OperationSpec {
    name: "client.create",
    group: "client",
    toolset: Toolset::Instances,
    summary: "Start a Client that owns Leases; it ends when its connection drops or it idles",
    http: Http {
        method: Method::Post,
        path: "/v1/clients",
    },
    cli: &["client", "create"],
    cli_positional: &[],
    mcp_tool: Some("client_create"),
    read_only: false,
    capability: None,
    input_schema: schema::<CreateInput>,
    output_schema: schema::<Client>,
};

static END: OperationSpec = OperationSpec {
    name: "client.end",
    group: "client",
    toolset: Toolset::Instances,
    summary: "End a Client and release its Leases",
    http: Http {
        method: Method::Delete,
        path: "/v1/clients/{client_id}",
    },
    cli: &["client", "end"],
    cli_positional: &["client_id"],
    mcp_tool: Some("client_end"),
    read_only: false,
    capability: None,
    input_schema: schema::<EndInput>,
    output_schema: schema::<Client>,
};

inventory::submit!(OpRegistration {
    spec: &CREATE,
    handler: create
});
inventory::submit!(OpRegistration {
    spec: &END,
    handler: end
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    kind: Option<ClientKind>,
    label: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EndInput {
    client_id: ClientId,
}

fn create(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: CreateInput = parse(input)?;
        let kind = input.kind.unwrap_or(ctx.client.kind);
        if kind == ClientKind::Connect {
            return Err(Error::InvalidArgument {
                field: "kind".to_string(),
                message: "connect Clients are created by the connect tunnel".to_string(),
            });
        }

        let label = input.label.unwrap_or_else(|| ctx.client.label.clone());

        let client = ctx.app.clients.create(&ctx.app, kind, label).await?;

        output(&client)
    }
    .boxed()
}

fn end(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: EndInput = parse(input)?;
        let client = ctx
            .app
            .clients
            .end(&ctx.app, &input.client_id, ClientEndReason::Ended)
            .await?;

        output(&client)
    }
    .boxed()
}
