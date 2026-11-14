use brr_core::{
    Error, EventData, Extension, ExtensionId, ExtensionSource, Http, Method, OperationSpec, Page,
    ProfileExtension, ProfileId, Toolset, Upload, schema,
};
use brr_store::{extension, profile, profile_extension};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, invalid, output, parse};
use crate::{
    app::Ctx,
    services::extensions::{self, AddSource, parse_amo_ref, parse_cws_ref},
};

static LIST: OperationSpec = OperationSpec {
    name: "extension.list",
    group: "extension",
    toolset: Toolset::Profiles,
    summary: "List the extension library with the Profiles that enable each",
    http: Http {
        method: Method::Get,
        path: "/v1/extensions",
    },
    cli: &["extension", "list"],
    cli_positional: &[],
    mcp_tool: Some("extension_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ListOutput>,
};

static ADD: OperationSpec = OperationSpec {
    name: "extension.add",
    group: "extension",
    toolset: Toolset::Profiles,
    summary: "Add an extension from the Chrome Web Store, addons.mozilla.org or an uploaded file or folder",
    http: Http {
        method: Method::Post,
        path: "/v1/extensions",
    },
    cli: &["extension", "add"],
    cli_positional: &["ref"],
    mcp_tool: Some("extension_add"),
    read_only: false,
    capability: None,
    input_schema: schema::<AddInput>,
    output_schema: schema::<Extension>,
};

static REMOVE: OperationSpec = OperationSpec {
    name: "extension.remove",
    group: "extension",
    toolset: Toolset::Profiles,
    summary: "Remove an extension and its files from the library",
    http: Http {
        method: Method::Delete,
        path: "/v1/extensions/{extension_id}",
    },
    cli: &["extension", "remove"],
    cli_positional: &["extension_id"],
    mcp_tool: Some("extension_remove"),
    read_only: false,
    capability: None,
    input_schema: schema::<ExtensionIdInput>,
    output_schema: schema::<Extension>,
};

static UPDATE: OperationSpec = OperationSpec {
    name: "extension.update",
    group: "extension",
    toolset: Toolset::Profiles,
    summary: "Fetch a newer version of a Web Store or add-ons extension",
    http: Http {
        method: Method::Post,
        path: "/v1/extensions/{extension_id}/update",
    },
    cli: &["extension", "update"],
    cli_positional: &["extension_id"],
    mcp_tool: Some("extension_update"),
    read_only: false,
    capability: None,
    input_schema: schema::<ExtensionIdInput>,
    output_schema: schema::<UpdateOutput>,
};

static PROFILE_GET: OperationSpec = OperationSpec {
    name: "profile.extensions_get",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "List the extensions a Profile enables",
    http: Http {
        method: Method::Get,
        path: "/v1/profiles/{profile}/extensions",
    },
    cli: &["profile", "extensions", "get"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_extensions_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<ProfileInput>,
    output_schema: schema::<ProfileExtensionsOutput>,
};

static PROFILE_SET: OperationSpec = OperationSpec {
    name: "profile.extensions_set",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Replace the extensions a Profile enables",
    http: Http {
        method: Method::Put,
        path: "/v1/profiles/{profile}/extensions",
    },
    cli: &["profile", "extensions", "set"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_extensions_set"),
    read_only: false,
    capability: None,
    input_schema: schema::<SetInput>,
    output_schema: schema::<ProfileExtensionsOutput>,
};

inventory::submit!(OpRegistration {
    spec: &LIST,
    handler: list
});
inventory::submit!(OpRegistration {
    spec: &ADD,
    handler: add
});
inventory::submit!(OpRegistration {
    spec: &REMOVE,
    handler: remove
});
inventory::submit!(OpRegistration {
    spec: &UPDATE,
    handler: update
});
inventory::submit!(OpRegistration {
    spec: &PROFILE_GET,
    handler: profile_get
});
inventory::submit!(OpRegistration {
    spec: &PROFILE_SET,
    handler: profile_set
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ProfileRef {
    id: ProfileId,
    name: String,
}

#[derive(Serialize, JsonSchema)]
struct ExtensionView {
    #[serde(flatten)]
    extension: Extension,
    latest_version: Option<String>,
    profiles: Vec<ProfileRef>,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    extensions: Vec<ExtensionView>,
    next_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddInput {
    source: Option<ExtensionSource>,
    #[serde(rename = "ref")]
    reference: Option<String>,
    file: Option<Upload>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExtensionIdInput {
    extension_id: ExtensionId,
}

#[derive(Serialize, JsonSchema)]
struct UpdateOutput {
    extension_id: ExtensionId,
    from: Option<String>,
    to: String,
    updated: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProfileInput {
    profile: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetInput {
    profile: String,
    extensions: Vec<ProfileExtension>,
}

#[derive(Serialize, JsonSchema)]
struct ProfileExtensionsOutput {
    extensions: Vec<ProfileExtension>,
}

fn add_source(input: AddInput) -> Result<AddSource, Error> {
    match (input.file, input.reference) {
        (Some(file), None) => match input.source {
            None | Some(ExtensionSource::Local) => Ok(AddSource::File { bytes: file.0 }),
            Some(_) => Err(invalid("source", "an uploaded file is a local extension")),
        },
        (None, Some(reference)) => match input.source {
            Some(ExtensionSource::Cws) => Ok(AddSource::Cws { reference }),
            Some(ExtensionSource::Amo) => Ok(AddSource::Amo { reference }),
            Some(ExtensionSource::Local) => {
                Err(invalid("source", "local extensions are added as a file"))
            }
            None if parse_cws_ref(&reference).is_some() => Ok(AddSource::Cws { reference }),
            None if parse_amo_ref(&reference, true).is_some() => Ok(AddSource::Amo { reference }),
            None => Err(invalid("source", "give source amo for a bare add-on slug")),
        },
        _ => Err(invalid("ref", "give exactly one of ref or file")),
    }
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let page = Page::new(input.limit, input.cursor);

        let (extensions, next_cursor) = ctx
            .app
            .store
            .run(move |c| {
                let (rows, next) = extension::list(c, &page)?;
                let views = rows
                    .into_iter()
                    .map(|row| {
                        let profiles = profile_extension::profiles_using(c, &row.id)?
                            .into_iter()
                            .map(|(id, name)| ProfileRef { id, name })
                            .collect();

                        Ok(ExtensionView {
                            latest_version: row.versions.first().map(|v| v.version.clone()),
                            extension: row,
                            profiles,
                        })
                    })
                    .collect::<Result<Vec<_>, Error>>()?;

                Ok((views, next))
            })
            .await?;

        output(&ListOutput {
            extensions,
            next_cursor,
        })
    }
    .boxed()
}

fn add(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: AddInput = parse(input)?;
        let added = extensions::add(&ctx.app, add_source(input)?).await?;

        output(&added)
    }
    .boxed()
}

fn remove(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ExtensionIdInput = parse(input)?;

        output(&extensions::remove(&ctx.app, &input.extension_id).await?)
    }
    .boxed()
}

fn update(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ExtensionIdInput = parse(input)?;
        let (from, to) = extensions::update(&ctx.app, &input.extension_id).await?;

        output(&UpdateOutput {
            extension_id: input.extension_id,
            updated: from.as_deref() != Some(to.as_str()),
            from,
            to,
        })
    }
    .boxed()
}

fn profile_get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ProfileInput = parse(input)?;
        let extensions = ctx
            .app
            .store
            .run(move |c| {
                let found = profile::find(c, &input.profile)?;

                profile_extension::get(c, &found.id)
            })
            .await?;

        output(&ProfileExtensionsOutput { extensions })
    }
    .boxed()
}

fn profile_set(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: SetInput = parse(input)?;

        let (profile_id, name) = ctx
            .app
            .store
            .run(move |c| {
                let found = profile::find(c, &input.profile)?;
                extensions::check_list(c, &input.extensions, &found.settings)?;
                profile_extension::set(c, &found.id, &input.extensions)?;

                Ok((found.id, found.name))
            })
            .await?;
        ctx.app
            .events
            .publish(EventData::ProfileUpdated { profile_id, name })
            .await?;

        let extensions = ctx
            .app
            .store
            .run(move |c| profile_extension::get(c, &profile_id))
            .await?;

        output(&ProfileExtensionsOutput { extensions })
    }
    .boxed()
}
