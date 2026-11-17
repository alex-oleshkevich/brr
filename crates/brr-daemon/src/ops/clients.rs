use brr_core::{
    Client, ClientEndReason, ClientId, ClientKind, Empty, Error, Http, Method, OperationSpec,
    Toolset, schema,
};
use brr_store::{client, lease};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, output, parse};
use crate::app::Ctx;

const CLIENTS_PATH: &str = "/v1/clients";

static LIST: OperationSpec = OperationSpec {
    name: "client.list",
    group: "client",
    toolset: Toolset::Instances,
    summary: "List the Clients that have not ended, with how many Leases each holds",
    http: Http {
        method: Method::Get,
        path: CLIENTS_PATH,
    },
    cli: &["client", "list"],
    cli_positional: &[],
    mcp_tool: Some("client_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<ListOutput>,
};

static CREATE: OperationSpec = OperationSpec {
    name: "client.create",
    group: "client",
    toolset: Toolset::Instances,
    summary: "Start a Client that owns Leases; it ends when its connection drops or it idles",
    http: Http {
        method: Method::Post,
        path: CLIENTS_PATH,
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
    spec: &LIST,
    handler: list
});
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

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    clients: Vec<ClientView>,
}

#[derive(Serialize, JsonSchema)]
struct ClientView {
    #[serde(flatten)]
    client: Client,
    lease_count: u32,
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        let (clients, leases) = ctx
            .app
            .store
            .run(|c| Ok((client::active(c)?, lease::active(c)?)))
            .await?;
        let clients = clients
            .into_iter()
            .map(|client| ClientView {
                lease_count: leases
                    .iter()
                    .filter(|lease| lease.client_id == client.id)
                    .count() as u32,
                client,
            })
            .collect();

        output(&ListOutput { clients })
    }
    .boxed()
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
