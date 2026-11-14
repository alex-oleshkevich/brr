use brr_core::{Backup, BackupId, Error, Http, Method, OperationSpec, TextUpload, Toolset, schema};
use brr_store::{backup, profile};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, output, parse};
use crate::{
    app::Ctx,
    services::backups::{self, Imported, passphrase},
};

static LIST: OperationSpec = OperationSpec {
    name: "backup.list",
    group: "backup",
    toolset: Toolset::Profiles,
    summary: "List a Profile's backups, newest first",
    http: Http {
        method: Method::Get,
        path: "/v1/profiles/{profile}/backups",
    },
    cli: &["backup", "list"],
    cli_positional: &["profile"],
    mcp_tool: Some("backup_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ProfileInput>,
    output_schema: schema::<ListOutput>,
};

static LIST_ALL: OperationSpec = OperationSpec {
    name: "backup.list_all",
    group: "backup",
    toolset: Toolset::Profiles,
    summary: "List every backup, newest first, including those of deleted Profiles",
    http: Http {
        method: Method::Get,
        path: "/v1/backups",
    },
    cli: &["backup", "list-all"],
    cli_positional: &[],
    mcp_tool: Some("backup_list_all"),
    read_only: true,
    capability: None,
    input_schema: schema::<NoInput>,
    output_schema: schema::<ListOutput>,
};

static CREATE: OperationSpec = OperationSpec {
    name: "backup.create",
    group: "backup",
    toolset: Toolset::Profiles,
    summary: "Back up a stopped Persistent Profile to the storage backend with the backup passphrase, then prune old copies",
    http: Http {
        method: Method::Post,
        path: "/v1/profiles/{profile}/backups",
    },
    cli: &["backup", "create"],
    cli_positional: &["profile"],
    mcp_tool: Some("backup_create"),
    read_only: false,
    capability: None,
    input_schema: schema::<ProfileInput>,
    output_schema: schema::<Backup>,
};

static RESTORE: OperationSpec = OperationSpec {
    name: "backup.restore",
    group: "backup",
    toolset: Toolset::Profiles,
    summary: "Restore a backup as a new Profile; the passphrase defaults to the backup passphrase",
    http: Http {
        method: Method::Post,
        path: "/v1/backups/{backup_id}/restore",
    },
    cli: &["backup", "restore"],
    cli_positional: &["backup_id"],
    mcp_tool: Some("backup_restore"),
    read_only: false,
    capability: None,
    input_schema: schema::<RestoreInput>,
    output_schema: schema::<Imported>,
};

static DELETE: OperationSpec = OperationSpec {
    name: "backup.delete",
    group: "backup",
    toolset: Toolset::Profiles,
    summary: "Delete a backup from the storage backend that holds it",
    http: Http {
        method: Method::Delete,
        path: "/v1/backups/{backup_id}",
    },
    cli: &["backup", "delete"],
    cli_positional: &["backup_id"],
    mcp_tool: Some("backup_delete"),
    read_only: false,
    capability: None,
    input_schema: schema::<DeleteInput>,
    output_schema: schema::<Backup>,
};

inventory::submit!(OpRegistration {
    spec: &LIST,
    handler: list
});
inventory::submit!(OpRegistration {
    spec: &LIST_ALL,
    handler: list_all
});
inventory::submit!(OpRegistration {
    spec: &CREATE,
    handler: create
});
inventory::submit!(OpRegistration {
    spec: &RESTORE,
    handler: restore
});
inventory::submit!(OpRegistration {
    spec: &DELETE,
    handler: delete
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProfileInput {
    profile: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoInput {}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    backups: Vec<Backup>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RestoreInput {
    backup_id: BackupId,
    passphrase: Option<TextUpload>,
    name: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteInput {
    backup_id: BackupId,
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ProfileInput = parse(input)?;
        let backups = ctx
            .app
            .store
            .run(move |c| backup::for_profile(c, &profile::find(c, &input.profile)?.id))
            .await?;

        output(&ListOutput { backups })
    }
    .boxed()
}

fn list_all(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let NoInput {} = parse(input)?;
        let backups = ctx.app.store.run(|c| backup::all(c)).await?;

        output(&ListOutput { backups })
    }
    .boxed()
}

fn create(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ProfileInput = parse(input)?;
        let found = ctx
            .app
            .store
            .run(move |c| profile::find(c, &input.profile))
            .await?;

        output(&backups::create(&ctx.app, &found, None).await?)
    }
    .boxed()
}

fn restore(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: RestoreInput = parse(input)?;
        let given = input
            .passphrase
            .map(|text| passphrase(&text.0))
            .transpose()?;

        output(&backups::restore(&ctx.app, input.backup_id, given, input.name).await?)
    }
    .boxed()
}

fn delete(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: DeleteInput = parse(input)?;
        let stored = ctx
            .app
            .store
            .run(move |c| backup::get(c, &input.backup_id))
            .await?;

        backups::remove(&ctx.app, &stored).await?;

        output(&stored)
    }
    .boxed()
}
