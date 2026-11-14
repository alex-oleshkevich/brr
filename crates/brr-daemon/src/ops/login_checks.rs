use brr_core::{
    ConflictReason, CredentialId, Error, Http, LoginCheck, LoginCheckId, LoginStatus, Method,
    OperationSpec, ProfileId, Toolset, schema, site_origin,
};
use brr_store::{credential, login_check, profile};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, invalid, output, parse, some};
use crate::app::Ctx;

const SITE_HELP: &str = "expected an http(s) origin, e.g. https://github.com";
const URL_HELP: &str = "expected an http:// or https:// URL";
const EXPECT_HELP: &str = "pass exactly one of expect_selector or expect_text";
const LINK_HELP: &str = "link the credential to this Profile first (credential.update profile_ids)";

static LIST: OperationSpec = OperationSpec {
    name: "login_check.list",
    group: "login_check",
    toolset: Toolset::Profiles,
    summary: "List a Profile's login checks with their last status",
    http: Http {
        method: Method::Get,
        path: "/v1/profiles/{profile}/login-checks",
    },
    cli: &["login-check", "list"],
    cli_positional: &["profile"],
    mcp_tool: Some("login_check_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ListOutput>,
};

static CREATE: OperationSpec = OperationSpec {
    name: "login_check.create",
    group: "login_check",
    toolset: Toolset::Profiles,
    summary: "Add a login check to a Profile: a URL that shows an element or text only when logged in",
    http: Http {
        method: Method::Post,
        path: "/v1/profiles/{profile}/login-checks",
    },
    cli: &["login-check", "create"],
    cli_positional: &["profile"],
    mcp_tool: Some("login_check_create"),
    read_only: false,
    capability: None,
    input_schema: schema::<CreateInput>,
    output_schema: schema::<LoginCheck>,
};

static UPDATE: OperationSpec = OperationSpec {
    name: "login_check.update",
    group: "login_check",
    toolset: Toolset::Profiles,
    summary: "Change a login check; omitted fields are kept and `credential_id: null` removes the credential",
    http: Http {
        method: Method::Patch,
        path: "/v1/login-checks/{login_check_id}",
    },
    cli: &["login-check", "update"],
    cli_positional: &["login_check_id"],
    mcp_tool: Some("login_check_update"),
    read_only: false,
    capability: None,
    input_schema: schema::<UpdateInput>,
    output_schema: schema::<LoginCheck>,
};

static DELETE: OperationSpec = OperationSpec {
    name: "login_check.delete",
    group: "login_check",
    toolset: Toolset::Profiles,
    summary: "Delete a login check",
    http: Http {
        method: Method::Delete,
        path: "/v1/login-checks/{login_check_id}",
    },
    cli: &["login-check", "delete"],
    cli_positional: &["login_check_id"],
    mcp_tool: Some("login_check_delete"),
    read_only: false,
    capability: None,
    input_schema: schema::<DeleteInput>,
    output_schema: schema::<DeleteOutput>,
};

static RUN: OperationSpec = OperationSpec {
    name: "login_check.run",
    group: "login_check",
    toolset: Toolset::Profiles,
    summary: "Run a login check now on the Profile's running Instance",
    http: Http {
        method: Method::Post,
        path: "/v1/login-checks/{login_check_id}/run",
    },
    cli: &["login-check", "run"],
    cli_positional: &["login_check_id"],
    mcp_tool: Some("login_check_run"),
    read_only: false,
    capability: None,
    input_schema: schema::<RunInput>,
    output_schema: schema::<LoginCheck>,
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
    spec: &RUN,
    handler: run
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    // Id or name.
    profile: String,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    login_checks: Vec<LoginCheck>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    profile: String,
    site: String,
    url: String,
    expect_selector: Option<String>,
    expect_text: Option<String>,
    credential_id: Option<CredentialId>,
    auto_relogin: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateInput {
    login_check_id: LoginCheckId,
    site: Option<String>,
    url: Option<String>,
    #[serde(default, deserialize_with = "some")]
    expect_selector: Option<Option<String>>,
    #[serde(default, deserialize_with = "some")]
    expect_text: Option<Option<String>>,
    #[serde(default, deserialize_with = "some")]
    credential_id: Option<Option<CredentialId>>,
    auto_relogin: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteInput {
    login_check_id: LoginCheckId,
}

#[derive(Serialize, JsonSchema)]
struct DeleteOutput {
    deleted: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RunInput {
    login_check_id: LoginCheckId,
}

// The whole check as it will be stored; the credential must exist and belong to the Profile.
fn validate(c: &brr_store::Connection, check: &mut LoginCheck) -> Result<(), Error> {
    check.site = site_origin(&check.site).ok_or_else(|| invalid("site", SITE_HELP))?;
    let lower = check.url.to_ascii_lowercase();
    if !lower.starts_with("http://") && !lower.starts_with("https://") {
        return Err(invalid("url", URL_HELP));
    }
    if check.expect_selector.is_some() == check.expect_text.is_some() {
        return Err(invalid("expect_selector", EXPECT_HELP));
    }
    if let Some(credential_id) = &check.credential_id {
        credential::get(c, credential_id)?;
        if !credential::is_linked(c, credential_id, &check.profile_id)? {
            return Err(Error::Conflict {
                reason: ConflictReason::CredentialNotLinked,
                detail: Some(LINK_HELP.to_owned()),
            });
        }
    }

    Ok(())
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let login_checks = ctx
            .app
            .store
            .run(move |c| {
                let found = profile::find(c, &input.profile)?;

                login_check::for_profile(c, &found.id)
            })
            .await?;

        output(&ListOutput { login_checks })
    }
    .boxed()
}

fn create(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: CreateInput = parse(input)?;
        let created = ctx
            .app
            .store
            .run(move |c| {
                let found: ProfileId = profile::find(c, &input.profile)?.id;
                let mut check = LoginCheck {
                    id: LoginCheckId::new(),
                    profile_id: found,
                    site: input.site,
                    url: input.url,
                    expect_selector: input.expect_selector,
                    expect_text: input.expect_text,
                    credential_id: input.credential_id,
                    auto_relogin: input.auto_relogin.unwrap_or(false),
                    last_status: LoginStatus::Unknown,
                    last_checked_at: None,
                };
                validate(c, &mut check)?;
                login_check::insert(c, &check)?;

                Ok(check)
            })
            .await?;

        output(&created)
    }
    .boxed()
}

fn update(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: UpdateInput = parse(input)?;
        let updated = ctx
            .app
            .store
            .run(move |c| {
                let mut check = login_check::get(c, &input.login_check_id)?;
                if let Some(site) = input.site {
                    check.site = site;
                }
                if let Some(url) = input.url {
                    check.url = url;
                }
                if let Some(selector) = input.expect_selector {
                    check.expect_selector = selector;
                }
                if let Some(text) = input.expect_text {
                    check.expect_text = text;
                }
                if let Some(credential_id) = input.credential_id {
                    check.credential_id = credential_id;
                }
                if let Some(auto_relogin) = input.auto_relogin {
                    check.auto_relogin = auto_relogin;
                }
                validate(c, &mut check)?;
                login_check::update(c, &check)?;

                Ok(check)
            })
            .await?;

        output(&updated)
    }
    .boxed()
}

fn delete(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: DeleteInput = parse(input)?;
        ctx.app
            .store
            .run(move |c| login_check::delete(c, &input.login_check_id))
            .await?;

        output(&DeleteOutput { deleted: true })
    }
    .boxed()
}

fn run(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: RunInput = parse(input)?;

        output(
            &ctx.app
                .login_checks
                .run(&ctx.app, &input.login_check_id)
                .await?,
        )
    }
    .boxed()
}
