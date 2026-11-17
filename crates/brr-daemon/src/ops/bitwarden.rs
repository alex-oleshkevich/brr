use brr_bitwarden::Credentials;
use brr_core::{
    BitwardenItem, BitwardenStatus, Credential, CredentialSelectors, Empty, Error, Http, Method,
    OperationSpec, Secret, Toolset, schema,
};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, output, parse};
use crate::{app::Ctx, services::bitwarden};

static STATUS: OperationSpec = OperationSpec {
    name: "bitwarden.status",
    group: "bitwarden",
    toolset: Toolset::Credentials,
    summary: "Show whether Bitwarden is connected and unlocked, its item count, last sync and last error",
    http: Http {
        method: Method::Get,
        path: "/v1/bitwarden/status",
    },
    cli: &["bitwarden", "status"],
    cli_positional: &[],
    mcp_tool: Some("bitwarden_status"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<BitwardenStatus>,
};

static CONNECT: OperationSpec = OperationSpec {
    name: "bitwarden.connect",
    group: "bitwarden",
    toolset: Toolset::Credentials,
    summary: "Connect a Bitwarden account with its API key and master password; they are kept only if the vault opens",
    http: Http {
        method: Method::Post,
        path: "/v1/bitwarden/connect",
    },
    cli: &["bitwarden", "connect"],
    cli_positional: &[],
    mcp_tool: Some("bitwarden_connect"),
    read_only: false,
    capability: None,
    input_schema: schema::<ConnectInput>,
    output_schema: schema::<BitwardenStatus>,
};

static SYNC: OperationSpec = OperationSpec {
    name: "bitwarden.sync",
    group: "bitwarden",
    toolset: Toolset::Credentials,
    summary: "Open the Bitwarden vault again now to pick up changed items",
    http: Http {
        method: Method::Post,
        path: "/v1/bitwarden/sync",
    },
    cli: &["bitwarden", "sync"],
    cli_positional: &[],
    mcp_tool: Some("bitwarden_sync"),
    read_only: false,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<BitwardenStatus>,
};

static ITEMS: OperationSpec = OperationSpec {
    name: "bitwarden.items",
    group: "bitwarden",
    toolset: Toolset::Credentials,
    summary: "List the Bitwarden login items, optionally only those whose name, URI or username contains `query`; no secrets",
    http: Http {
        method: Method::Get,
        path: "/v1/bitwarden/items",
    },
    cli: &["bitwarden", "items"],
    cli_positional: &["query"],
    mcp_tool: Some("bitwarden_items"),
    read_only: true,
    capability: None,
    input_schema: schema::<ItemsInput>,
    output_schema: schema::<ItemsOutput>,
};

static LINK: OperationSpec = OperationSpec {
    name: "bitwarden.link",
    group: "bitwarden",
    toolset: Toolset::Credentials,
    summary: "Make a Bitwarden login item a credential for the given Profiles; an item already linked gains the Profiles",
    http: Http {
        method: Method::Post,
        path: "/v1/bitwarden/link",
    },
    cli: &["bitwarden", "link"],
    cli_positional: &["item_id"],
    mcp_tool: Some("bitwarden_link"),
    read_only: false,
    capability: None,
    input_schema: schema::<LinkInput>,
    output_schema: schema::<Credential>,
};

static DISCONNECT: OperationSpec = OperationSpec {
    name: "bitwarden.disconnect",
    group: "bitwarden",
    toolset: Toolset::Credentials,
    summary: "Forget the Bitwarden API key and master password; linked credentials stay and fail as locked until you connect again",
    http: Http {
        method: Method::Post,
        path: "/v1/bitwarden/disconnect",
    },
    cli: &["bitwarden", "disconnect"],
    cli_positional: &[],
    mcp_tool: Some("bitwarden_disconnect"),
    read_only: false,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<BitwardenStatus>,
};

inventory::submit!(OpRegistration {
    spec: &STATUS,
    handler: status
});
inventory::submit!(OpRegistration {
    spec: &CONNECT,
    handler: connect
});
inventory::submit!(OpRegistration {
    spec: &SYNC,
    handler: sync
});
inventory::submit!(OpRegistration {
    spec: &ITEMS,
    handler: items
});
inventory::submit!(OpRegistration {
    spec: &LINK,
    handler: link
});
inventory::submit!(OpRegistration {
    spec: &DISCONNECT,
    handler: disconnect
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ConnectInput {
    // Absent: the configured bitwarden.server_url.
    server_url: Option<String>,
    client_id: String,
    client_secret: Secret,
    master_password: Secret,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ItemsInput {
    query: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ItemsOutput {
    items: Vec<BitwardenItem>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LinkInput {
    item_id: String,
    profile_ids: Option<Vec<String>>,
    selectors: Option<CredentialSelectors>,
}

fn status(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        output(&bitwarden::status(&ctx.app).await?)
    }
    .boxed()
}

fn connect(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ConnectInput = parse(input)?;
        let server_url = input
            .server_url
            .unwrap_or_else(|| ctx.app.settings().bitwarden.server_url.clone());
        let creds = Credentials {
            client_id: input.client_id,
            client_secret: input.client_secret,
            master_password: input.master_password,
        };

        output(&bitwarden::connect(&ctx.app, &server_url, creds).await?)
    }
    .boxed()
}

fn sync(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        output(&bitwarden::sync(&ctx.app).await?)
    }
    .boxed()
}

fn items(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ItemsInput = parse(input)?;
        let items = bitwarden::items(&ctx.app, input.query.as_deref()).await?;

        output(&ItemsOutput { items })
    }
    .boxed()
}

fn link(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: LinkInput = parse(input)?;
        let credential = bitwarden::link(
            &ctx.app,
            input.item_id,
            input.profile_ids.unwrap_or_default(),
            input.selectors,
        )
        .await?;

        output(&credential)
    }
    .boxed()
}

fn disconnect(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        output(&bitwarden::disconnect(&ctx.app).await?)
    }
    .boxed()
}
