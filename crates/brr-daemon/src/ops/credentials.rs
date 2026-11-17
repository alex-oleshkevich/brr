use std::sync::Arc;

use brr_core::{
    Credential, CredentialId, CredentialSelectors, CredentialSource, Error, Http, Method,
    OperationSpec, Page, PasswordCsvFormat, Secret, TextUpload, Toolset, now_ms, schema,
    site_origin,
};
use brr_store::credential;
use brr_vault::totp::totp_check;
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, invalid, output, parse, some};
use crate::{
    app::Ctx,
    internal,
    services::credentials::{
        self, NOTES, NewCredential, PASSWORD, TOTP, USERNAME, resolve_profiles, seal_field,
    },
};

const SITE_HELP: &str = "expected a site such as github.com or https://github.com";
const LOGIN_URL_HELP: &str = "expected an http:// or https:// URL";
const BITWARDEN_FIELD: &str = "comes from the linked Bitwarden item; change it in Bitwarden";

static LIST: OperationSpec = OperationSpec {
    name: "credential.list",
    group: "credential",
    toolset: Toolset::Credentials,
    summary: "List vault credentials by site; passwords, TOTP secrets and notes are never returned",
    http: Http {
        method: Method::Get,
        path: "/v1/credentials",
    },
    cli: &["credential", "list"],
    cli_positional: &[],
    mcp_tool: Some("credential_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ListOutput>,
};

static CREATE: OperationSpec = OperationSpec {
    name: "credential.create",
    group: "credential",
    toolset: Toolset::Credentials,
    summary: "Store a login in the vault, optionally linked to Profiles by id or name",
    http: Http {
        method: Method::Post,
        path: "/v1/credentials",
    },
    cli: &["credential", "create"],
    cli_positional: &["site"],
    mcp_tool: Some("credential_create"),
    read_only: false,
    capability: None,
    input_schema: schema::<CreateInput>,
    output_schema: schema::<Credential>,
};

static UPDATE: OperationSpec = OperationSpec {
    name: "credential.update",
    group: "credential",
    toolset: Toolset::Credentials,
    summary: "Change a credential; omitted fields are kept, `totp: null` removes the TOTP secret, `login_url: null` falls back to the site and `profile_ids` replaces the links",
    http: Http {
        method: Method::Patch,
        path: "/v1/credentials/{credential_id}",
    },
    cli: &["credential", "update"],
    cli_positional: &["credential_id"],
    mcp_tool: Some("credential_update"),
    read_only: false,
    capability: None,
    input_schema: schema::<UpdateInput>,
    output_schema: schema::<Credential>,
};

static DELETE: OperationSpec = OperationSpec {
    name: "credential.delete",
    group: "credential",
    toolset: Toolset::Credentials,
    summary: "Delete a credential from the vault",
    http: Http {
        method: Method::Delete,
        path: "/v1/credentials/{credential_id}",
    },
    cli: &["credential", "delete"],
    cli_positional: &["credential_id"],
    mcp_tool: Some("credential_delete"),
    read_only: false,
    capability: None,
    input_schema: schema::<DeleteInput>,
    output_schema: schema::<DeleteOutput>,
};

static IMPORT: OperationSpec = OperationSpec {
    name: "credential.import",
    group: "credential",
    toolset: Toolset::Credentials,
    summary: "Import a Chrome or Firefox passwords CSV; logins already in the vault are skipped",
    http: Http {
        method: Method::Post,
        path: "/v1/credentials/import",
    },
    cli: &["credential", "import"],
    cli_positional: &["file"],
    mcp_tool: Some("credential_import"),
    read_only: false,
    capability: None,
    input_schema: schema::<ImportInput>,
    output_schema: schema::<ImportOutput>,
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
    spec: &UPDATE,
    handler: update
});
inventory::submit!(OpRegistration {
    spec: &DELETE,
    handler: delete
});
inventory::submit!(OpRegistration {
    spec: &IMPORT,
    handler: import
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    credentials: Vec<Credential>,
    next_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    site: String,
    login_url: Option<String>,
    username: String,
    password: Secret,
    totp: Option<Secret>,
    notes: Option<String>,
    selectors: Option<CredentialSelectors>,
    profile_ids: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateInput {
    credential_id: CredentialId,
    site: Option<String>,
    #[serde(default, deserialize_with = "some")]
    login_url: Option<Option<String>>,
    username: Option<String>,
    password: Option<Secret>,
    #[serde(default, deserialize_with = "some")]
    totp: Option<Option<Secret>>,
    notes: Option<String>,
    selectors: Option<CredentialSelectors>,
    profile_ids: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteInput {
    credential_id: CredentialId,
}

#[derive(Serialize, JsonSchema)]
struct DeleteOutput {
    deleted: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportInput {
    format: PasswordCsvFormat,
    file: TextUpload,
    profile_ids: Option<Vec<String>>,
}

#[derive(Serialize, JsonSchema)]
struct ImportOutput {
    imported: u32,
    skipped: u32,
}

// A bare host ("github.com") is taken as https.
fn validate_site(site: &str) -> Result<String, Error> {
    let site = site.trim();
    let url = if site.contains("://") {
        site.to_owned()
    } else {
        format!("https://{site}")
    };

    site_origin(&url).ok_or_else(|| invalid("site", SITE_HELP))
}

fn validate_username(username: &str) -> Result<(), Error> {
    if username.is_empty() {
        return Err(invalid("username", "must not be empty"));
    }

    Ok(())
}

fn validate_login_url(login_url: String) -> Result<String, Error> {
    let lower = login_url.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return Err(invalid("login_url", LOGIN_URL_HELP));
    }

    Ok(login_url)
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let page = Page::new(input.limit, input.cursor);

        let (rows, next_cursor, mut last_logins) = ctx
            .app
            .store
            .run(move |c| {
                let (found, next) = credential::list(c, &page)?;
                let rows = found
                    .into_iter()
                    .map(|row| credential::profiles(c, &row.id).map(|ids| (row, ids)))
                    .collect::<Result<Vec<_>, _>>()?;

                Ok((rows, next, credential::last_logins(c)?))
            })
            .await?;

        let credentials = rows
            .into_iter()
            .map(|(row, profile_ids)| {
                let last_login = last_logins.remove(&row.id);

                credentials::view(&ctx.app, &row, profile_ids, last_login)
            })
            .collect::<Result<Vec<_>, _>>()?;

        output(&ListOutput {
            credentials,
            next_cursor,
        })
    }
    .boxed()
}

fn create(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: CreateInput = parse(input)?;
        validate_username(&input.username)?;
        let new = NewCredential {
            site: validate_site(&input.site)?,
            login_url: input.login_url.map(validate_login_url).transpose()?,
            username: input.username,
            password: input.password,
            totp: input.totp,
            notes: input.notes,
            selectors: input.selectors.unwrap_or_default(),
        };
        if let Some(totp) = &new.totp {
            totp_check(totp.expose())?;
        }
        let credential_id = CredentialId::new();
        let row = credentials::new_row(&ctx.app.secrets, credential_id, &new)?;
        let profiles = input.profile_ids.unwrap_or_default();
        let app = Arc::clone(&ctx.app);
        ctx.app
            .store
            .run(move |c| {
                credentials::ensure_unique(&app.secrets, c, &new.site, &new.username, None)?;
                let linked = resolve_profiles(c, &profiles)?;

                let tx = c.transaction().map_err(internal)?;
                credential::insert(&tx, &row)?;
                credential::set_profiles(&tx, &row.id, &linked)?;
                tx.commit().map_err(internal)
            })
            .await?;

        output(&credentials::load(&ctx.app, &credential_id).await?)
    }
    .boxed()
}

fn update(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: UpdateInput = parse(input)?;
        let credential_id = input.credential_id;
        let site = input.site.as_deref().map(validate_site).transpose()?;
        let login_url = input
            .login_url
            .map(|url| url.map(validate_login_url).transpose())
            .transpose()?;
        if let Some(Some(totp)) = &input.totp {
            totp_check(totp.expose())?;
        }
        input
            .username
            .as_deref()
            .map(validate_username)
            .transpose()?;
        let secrets = &ctx.app.secrets;
        let username_enc = input
            .username
            .as_deref()
            .map(|v| seal_field(secrets, &credential_id, USERNAME, v))
            .transpose()?;
        let password_enc = input
            .password
            .as_ref()
            .map(|v| seal_field(secrets, &credential_id, PASSWORD, v.expose()))
            .transpose()?;
        let totp_enc = input
            .totp
            .as_ref()
            .map(|totp| {
                totp.as_ref()
                    .map(|v| seal_field(secrets, &credential_id, TOTP, v.expose()))
                    .transpose()
            })
            .transpose()?;
        let notes_enc = input
            .notes
            .as_deref()
            .map(|v| seal_field(secrets, &credential_id, NOTES, v))
            .transpose()?;
        let selectors = input.selectors;
        let profiles = input.profile_ids;
        let username = input.username;
        let app = Arc::clone(&ctx.app);

        ctx.app
            .store
            .run(move |c| {
                let mut row = credential::get(c, &credential_id)?;
                let from_bitwarden = [
                    ("site", site.is_some()),
                    ("username", username.is_some()),
                    ("password", password_enc.is_some()),
                    ("totp", totp_enc.is_some()),
                ]
                .into_iter()
                .find(|(_, given)| *given && row.source == CredentialSource::Bitwarden);
                if let Some((field, _)) = from_bitwarden {
                    return Err(invalid(field, BITWARDEN_FIELD));
                }
                if site.is_some() || username.is_some() {
                    let current = match username {
                        Some(username) => username,
                        None => credentials::open_username(&app.secrets, &row)?,
                    };
                    let checked_site = site.as_deref().unwrap_or(&row.site);

                    credentials::ensure_unique(
                        &app.secrets,
                        c,
                        checked_site,
                        &current,
                        Some(&credential_id),
                    )?;
                }
                row.site = site.unwrap_or(row.site);
                row.login_url = login_url.unwrap_or(row.login_url);
                row.username_enc = username_enc.or(row.username_enc);
                row.password_enc = password_enc.or(row.password_enc);
                row.totp_enc = totp_enc.unwrap_or(row.totp_enc);
                row.notes_enc = notes_enc.or(row.notes_enc);
                row.selectors = selectors.unwrap_or(row.selectors);
                row.updated_at = now_ms();
                let linked = profiles
                    .map(|profiles| resolve_profiles(c, &profiles))
                    .transpose()?;

                let tx = c.transaction().map_err(internal)?;
                credential::update(&tx, &row)?;
                if let Some(linked) = &linked {
                    credential::set_profiles(&tx, &row.id, linked)?;
                }
                tx.commit().map_err(internal)
            })
            .await?;

        output(&credentials::load(&ctx.app, &credential_id).await?)
    }
    .boxed()
}

fn delete(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: DeleteInput = parse(input)?;
        let credential_id = input.credential_id;
        ctx.app
            .store
            .run(move |c| credential::delete(c, &credential_id))
            .await?;

        output(&DeleteOutput { deleted: true })
    }
    .boxed()
}

fn import(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ImportInput = parse(input)?;
        let (imported, skipped) = credentials::import(
            &ctx.app,
            input.format,
            input.file.0,
            input.profile_ids.unwrap_or_default(),
        )
        .await?;

        output(&ImportOutput { imported, skipped })
    }
    .boxed()
}
