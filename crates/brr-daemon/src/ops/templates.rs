use brr_core::{
    ConflictReason, Error, Http, Method, OperationSpec, Page, ProfileExtension, ProfileSettings,
    Template, TemplateId, Toolset, now_ms, schema,
};
use brr_store::{profile, profile_extension, template};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, invalid, output, parse, some};
use crate::{
    app::Ctx,
    services::{extensions, profiles::validate_name},
};

const MAX_DESCRIPTION_CHARS: usize = 200;

static LIST: OperationSpec = OperationSpec {
    name: "template.list",
    group: "template",
    toolset: Toolset::Profiles,
    summary: "List built-in and user templates by name",
    http: Http {
        method: Method::Get,
        path: "/v1/templates",
    },
    cli: &["template", "list"],
    cli_positional: &[],
    mcp_tool: Some("template_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ListOutput>,
};

static GET: OperationSpec = OperationSpec {
    name: "template.get",
    group: "template",
    toolset: Toolset::Profiles,
    summary: "Show one template by id or name",
    http: Http {
        method: Method::Get,
        path: "/v1/templates/{template}",
    },
    cli: &["template", "get"],
    cli_positional: &["template"],
    mcp_tool: Some("template_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<GetInput>,
    output_schema: schema::<Template>,
};

static CREATE: OperationSpec = OperationSpec {
    name: "template.create",
    group: "template",
    toolset: Toolset::Profiles,
    summary: "Create a user template from settings and the extensions it hands to new Profiles",
    http: Http {
        method: Method::Post,
        path: "/v1/templates",
    },
    cli: &["template", "create"],
    cli_positional: &["name"],
    mcp_tool: Some("template_create"),
    read_only: false,
    capability: None,
    input_schema: schema::<CreateInput>,
    output_schema: schema::<Template>,
};

static UPDATE: OperationSpec = OperationSpec {
    name: "template.update",
    group: "template",
    toolset: Toolset::Profiles,
    summary: "Change a user template; existing Profiles keep their copy of the settings",
    http: Http {
        method: Method::Patch,
        path: "/v1/templates/{template}",
    },
    cli: &["template", "update"],
    cli_positional: &["template"],
    mcp_tool: Some("template_update"),
    read_only: false,
    capability: None,
    input_schema: schema::<UpdateInput>,
    output_schema: schema::<Template>,
};

static DELETE: OperationSpec = OperationSpec {
    name: "template.delete",
    group: "template",
    toolset: Toolset::Profiles,
    summary: "Delete a user template; built-in templates cannot be deleted",
    http: Http {
        method: Method::Delete,
        path: "/v1/templates/{template}",
    },
    cli: &["template", "delete"],
    cli_positional: &["template"],
    mcp_tool: Some("template_delete"),
    read_only: false,
    capability: None,
    input_schema: schema::<GetInput>,
    output_schema: schema::<Template>,
};

static SAVE_FROM_PROFILE: OperationSpec = OperationSpec {
    name: "template.save_from_profile",
    group: "template",
    toolset: Toolset::Profiles,
    summary: "Save a Profile's settings and extensions as a new user template",
    http: Http {
        method: Method::Post,
        path: "/v1/profiles/{profile}/save-as-template",
    },
    cli: &["template", "save-from-profile"],
    cli_positional: &["profile"],
    mcp_tool: Some("template_save_from_profile"),
    read_only: false,
    capability: None,
    input_schema: schema::<SaveInput>,
    output_schema: schema::<Template>,
};

inventory::submit!(OpRegistration {
    spec: &LIST,
    handler: list
});
inventory::submit!(OpRegistration {
    spec: &GET,
    handler: get
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
    spec: &SAVE_FROM_PROFILE,
    handler: save_from_profile
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    templates: Vec<Template>,
    next_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetInput {
    template: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    name: String,
    description: Option<String>,
    settings: ProfileSettings,
    #[serde(default)]
    extensions: Vec<ProfileExtension>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateInput {
    template: String,
    name: Option<String>,
    #[serde(default, deserialize_with = "some")]
    description: Option<Option<String>>,
    settings: Option<ProfileSettings>,
    extensions: Option<Vec<ProfileExtension>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SaveInput {
    profile: String,
    name: String,
}

fn validate_description(description: &str) -> Result<(), Error> {
    if description.chars().count() > MAX_DESCRIPTION_CHARS
        || description.chars().any(char::is_control)
    {
        return Err(invalid(
            "description",
            &format!("use one line of at most {MAX_DESCRIPTION_CHARS} characters"),
        ));
    }

    Ok(())
}

fn user_template(
    name: String,
    description: Option<String>,
    settings: ProfileSettings,
    extensions: Vec<ProfileExtension>,
) -> Result<Template, Error> {
    validate_name(&name, "name")?;
    description
        .as_deref()
        .map(validate_description)
        .transpose()?;
    settings.validate()?;
    let now = now_ms();

    Ok(Template {
        id: TemplateId::new(),
        name,
        builtin: false,
        description,
        settings,
        extensions,
        created_at: now,
        updated_at: now,
    })
}

fn refuse_builtin(found: &Template) -> Result<(), Error> {
    if found.builtin {
        return Err(Error::Conflict {
            reason: ConflictReason::Builtin,
            detail: None,
        });
    }

    Ok(())
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let page = Page::new(input.limit, input.cursor);

        let (templates, next_cursor) = ctx.app.store.run(move |c| template::list(c, &page)).await?;

        output(&ListOutput {
            templates,
            next_cursor,
        })
    }
    .boxed()
}

fn get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: GetInput = parse(input)?;
        let found = ctx
            .app
            .store
            .run(move |c| template::find(c, &input.template))
            .await?;

        output(&found)
    }
    .boxed()
}

fn create(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: CreateInput = parse(input)?;
        let created = user_template(
            input.name,
            input.description,
            input.settings,
            input.extensions,
        )?;

        let stored = ctx
            .app
            .store
            .run(move |c| {
                extensions::check_list(c, &created.extensions, &created.settings)?;
                template::insert(c, &created)?;

                template::get(c, &created.id)
            })
            .await?;

        output(&stored)
    }
    .boxed()
}

fn update(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: UpdateInput = parse(input)?;
        if let Some(name) = &input.name {
            validate_name(name, "name")?;
        }
        if let Some(settings) = &input.settings {
            settings.validate()?;
        }
        if let Some(Some(description)) = &input.description {
            validate_description(description)?;
        }

        let updated = ctx
            .app
            .store
            .run(move |c| {
                let mut found = template::find(c, &input.template)?;
                refuse_builtin(&found)?;
                let recheck = input.settings.is_some() || input.extensions.is_some();
                found.name = input.name.unwrap_or(found.name);
                found.description = input.description.unwrap_or(found.description);
                found.settings = input.settings.unwrap_or(found.settings);
                found.extensions = input.extensions.unwrap_or(found.extensions);
                if recheck {
                    extensions::check_list(c, &found.extensions, &found.settings)?;
                }
                found.updated_at = now_ms();
                template::update(c, &found)?;

                template::get(c, &found.id)
            })
            .await?;

        output(&updated)
    }
    .boxed()
}

fn delete(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: GetInput = parse(input)?;
        let deleted = ctx
            .app
            .store
            .run(move |c| {
                let found = template::find(c, &input.template)?;
                refuse_builtin(&found)?;
                template::delete(c, &found.id)?;

                Ok(found)
            })
            .await?;

        output(&deleted)
    }
    .boxed()
}

fn save_from_profile(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: SaveInput = parse(input)?;

        let saved = ctx
            .app
            .store
            .run(move |c| {
                let source = profile::find(c, &input.profile)?;
                let enabled = profile_extension::get(c, &source.id)?;
                let mut saved = user_template(input.name, None, source.settings, enabled)?;
                saved.description = Some(source.notes).filter(|notes| !notes.is_empty());
                extensions::check_list(c, &saved.extensions, &saved.settings)?;
                template::insert(c, &saved)?;

                template::get(c, &saved.id)
            })
            .await?;

        output(&saved)
    }
    .boxed()
}
