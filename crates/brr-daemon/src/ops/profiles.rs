use std::{collections::BTreeSet, fs};

use brr_core::{
    BackupSchedule, ConflictReason, Error, EventData, ExtensionId, Http, Method, OperationSpec,
    Page, Profile, ProfileId, ProfileKind, ProfileSettings, Toolset, now_ms, schema,
};
use brr_store::{build, instance, login_check, profile, profile_extension, template};
use futures::{FutureExt, future::BoxFuture};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{OpRegistration, invalid, output, parse, some};
use crate::{
    app::Ctx,
    internal,
    services::{
        backups::stored_passphrase,
        events, extensions,
        profiles::{
            DEFAULT_BACKUP_KEEP, mark_builtin, removable_data_dir, validate_name,
            with_capabilities, with_login_status,
        },
        proxies,
        relay::companion,
    },
};

static LIST: OperationSpec = OperationSpec {
    name: "profile.list",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "List Profiles by name, optionally only those with a tag",
    http: Http {
        method: Method::Get,
        path: "/v1/profiles",
    },
    cli: &["profile", "list"],
    cli_positional: &[],
    mcp_tool: Some("profile_list"),
    read_only: true,
    capability: None,
    input_schema: schema::<ListInput>,
    output_schema: schema::<ListOutput>,
};

static GET: OperationSpec = OperationSpec {
    name: "profile.get",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Show one Profile by id or name, with the Capabilities of its Build",
    http: Http {
        method: Method::Get,
        path: "/v1/profiles/{profile}",
    },
    cli: &["profile", "get"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<GetInput>,
    output_schema: schema::<Profile>,
};

static CREATE: OperationSpec = OperationSpec {
    name: "profile.create",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Create a Profile from a template; it copies the template's settings",
    http: Http {
        method: Method::Post,
        path: "/v1/profiles",
    },
    cli: &["profile", "create"],
    cli_positional: &["name"],
    mcp_tool: Some("profile_create"),
    read_only: false,
    capability: None,
    input_schema: schema::<CreateInput>,
    output_schema: schema::<Profile>,
};

static UPDATE: OperationSpec = OperationSpec {
    name: "profile.update",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Change a Profile; `settings` replaces the whole settings object",
    http: Http {
        method: Method::Patch,
        path: "/v1/profiles/{profile}",
    },
    cli: &["profile", "update"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_update"),
    read_only: false,
    capability: None,
    input_schema: schema::<UpdateInput>,
    output_schema: schema::<Profile>,
};

static DELETE: OperationSpec = OperationSpec {
    name: "profile.delete",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Delete a Profile and its data folder; refused while it is running",
    http: Http {
        method: Method::Delete,
        path: "/v1/profiles/{profile}",
    },
    cli: &["profile", "delete"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_delete"),
    read_only: false,
    capability: None,
    input_schema: schema::<GetInput>,
    output_schema: schema::<Profile>,
};

static REAPPLY_TEMPLATE: OperationSpec = OperationSpec {
    name: "profile.reapply_template",
    group: "profile",
    toolset: Toolset::Profiles,
    summary: "Show how a Profile's settings and extensions differ from its template now, and copy the template's over them unless `dry_run` is set; refused while the Profile runs",
    http: Http {
        method: Method::Post,
        path: "/v1/profiles/{profile}/reapply-template",
    },
    cli: &["profile", "reapply-template"],
    cli_positional: &["profile"],
    mcp_tool: Some("profile_reapply_template"),
    read_only: false,
    capability: None,
    input_schema: schema::<ReapplyInput>,
    output_schema: schema::<ReapplyOutput>,
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
    spec: &REAPPLY_TEMPLATE,
    handler: reapply_template
});

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {
    tag: Option<String>,
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ListOutput {
    profiles: Vec<Profile>,
    next_cursor: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetInput {
    profile: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    name: String,
    template: String,
    kind: Option<ProfileKind>,
    tags: Option<Vec<String>>,
    notes: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateInput {
    profile: String,
    name: Option<String>,
    settings: Option<ProfileSettings>,
    pinned: Option<bool>,
    notes: Option<String>,
    tags: Option<Vec<String>>,
    #[serde(default, deserialize_with = "some")]
    disk_quota_bytes: Option<Option<u64>>,
    #[serde(default, deserialize_with = "some")]
    backup_schedule: Option<Option<String>>,
    backup_keep: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReapplyInput {
    profile: String,
    dry_run: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
struct ReapplyOutput {
    changes: Vec<FieldChange>,
    extensions: ExtensionChanges,
}

#[derive(Serialize, JsonSchema)]
struct FieldChange {
    field: String,
    from: Value,
    to: Value,
}

#[derive(Serialize, JsonSchema)]
struct ExtensionChanges {
    added: Vec<ExtensionId>,
    removed: Vec<ExtensionId>,
}

fn validate_tags(tags: &[String]) -> Result<(), Error> {
    tags.iter().try_for_each(|tag| validate_name(tag, "tags"))
}

fn list(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ListInput = parse(input)?;
        let page = Page::new(input.limit, input.cursor);

        let (profiles, next_cursor) = ctx
            .app
            .store
            .run(move |c| {
                let (found, next) = profile::list(c, input.tag.as_deref(), &page)?;
                let builds = build::all(c)?;
                let statuses = login_check::profile_statuses(c)?;
                let profiles: Vec<Profile> = found
                    .into_iter()
                    .map(|p| with_login_status(&statuses, with_capabilities(&builds, p)))
                    .collect();

                Ok((profiles, next))
            })
            .await?;

        let settings = ctx.app.settings();
        let profiles = profiles
            .into_iter()
            .map(|found| mark_builtin(&settings, found))
            .collect();

        output(&ListOutput {
            profiles,
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
            .run(move |c| {
                Ok(with_login_status(
                    &login_check::profile_statuses(c)?,
                    with_capabilities(&build::all(c)?, profile::find(c, &input.profile)?),
                ))
            })
            .await?;

        output(&mark_builtin(&ctx.app.settings(), found))
    }
    .boxed()
}

fn create(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: CreateInput = parse(input)?;
        let tags = input.tags.unwrap_or_default();
        validate_name(&input.name, "name")?;
        validate_tags(&tags)?;

        let profiles_dir = ctx.app.paths.profiles_dir();
        let kind = input.kind.unwrap_or(ProfileKind::Persistent);
        let created = ctx
            .app
            .store
            .run(move |c| {
                let template = template::find(c, &input.template)?;
                template.settings.validate()?;
                companion::check_settings(c, &template.settings)?;
                proxies::ensure_exists(c, template.settings.proxy.as_ref())?;
                extensions::check_list(c, &template.extensions, &template.settings)?;

                let id = ProfileId::new();
                let now = now_ms();
                let created = Profile {
                    data_dir: (kind == ProfileKind::Persistent)
                        .then(|| profiles_dir.join(id.to_string())),
                    id,
                    name: input.name,
                    kind,
                    template_id: Some(template.id),
                    template_name: Some(template.name),
                    settings: template.settings,
                    pinned: false,
                    notes: input.notes.unwrap_or_default(),
                    tags,
                    disk_quota_bytes: None,
                    backup_schedule: None,
                    backup_keep: DEFAULT_BACKUP_KEEP,
                    created_at: now,
                    updated_at: now,
                    capabilities: Vec::new(),
                    login_status: None,
                    builtin: false,
                };
                profile::insert(c, &created)?;
                profile_extension::set(c, &created.id, &template.extensions)?;

                if let Some(dir) = &created.data_dir
                    && let Err(err) = fs::create_dir_all(dir)
                {
                    profile::delete(c, &created.id)?;

                    return Err(internal(err));
                }

                Ok(with_capabilities(&build::all(c)?, created))
            })
            .await?;

        ctx.app
            .events
            .publish(EventData::ProfileCreated {
                profile_id: created.id,
                name: created.name.clone(),
            })
            .await?;

        output(&mark_builtin(&ctx.app.settings(), created))
    }
    .boxed()
}

fn update(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: UpdateInput = parse(input)?;
        if let Some(name) = &input.name {
            validate_name(name, "name")?;
        }
        if let Some(tags) = &input.tags {
            validate_tags(tags)?;
        }
        if let Some(settings) = &input.settings {
            settings.validate()?;
        }

        if input.backup_keep == Some(0) {
            return Err(invalid("backup_keep", "must be at least 1"));
        }
        // Stored in its canonical spelling, so the scheduler and the UI read one form.
        let schedule = match &input.backup_schedule {
            Some(Some(text)) => {
                let schedule = text.parse::<BackupSchedule>()?;
                if stored_passphrase(&ctx.app).await?.is_none() {
                    return Err(invalid(
                        "backup_schedule",
                        "set a backup passphrase first (settings set backup.passphrase)",
                    ));
                }

                Some(Some(schedule.to_string()))
            }
            Some(None) => Some(None),
            None => None,
        };

        let settings_given = input.settings.is_some();
        let (updated, idle_changed) = ctx
            .app
            .store
            .run(move |c| {
                let mut found = profile::find(c, &input.profile)?;
                let was_pinned = found.pinned;
                let was_idle = found.settings.idle.clone();
                found.name = input.name.unwrap_or(found.name);
                found.settings = input.settings.unwrap_or(found.settings);
                proxies::ensure_exists(c, found.settings.proxy.as_ref())?;
                if settings_given {
                    let enabled = profile_extension::get(c, &found.id)?;
                    extensions::check_list(c, &enabled, &found.settings)?;
                    companion::check_settings(c, &found.settings)?;
                }
                found.pinned = input.pinned.unwrap_or(found.pinned);
                found.notes = input.notes.unwrap_or(found.notes);
                found.tags = input.tags.unwrap_or(found.tags);
                found.disk_quota_bytes = input.disk_quota_bytes.unwrap_or(found.disk_quota_bytes);
                if matches!(schedule, Some(Some(_))) && found.kind != ProfileKind::Persistent {
                    return Err(invalid(
                        "backup_schedule",
                        "only Persistent profiles have data to back up",
                    ));
                }
                found.backup_schedule = schedule.unwrap_or(found.backup_schedule);
                found.backup_keep = input.backup_keep.unwrap_or(found.backup_keep);
                found.updated_at = now_ms();
                profile::update(c, &found)?;
                let idle_changed = found.pinned != was_pinned || found.settings.idle != was_idle;

                Ok((with_capabilities(&build::all(c)?, found), idle_changed))
            })
            .await?;

        if idle_changed {
            ctx.app
                .instances
                .profile_changed(&updated.id, updated.pinned, updated.settings.idle.clone())
                .await;
        }
        ctx.app
            .events
            .publish(EventData::ProfileUpdated {
                profile_id: updated.id,
                name: updated.name.clone(),
            })
            .await?;

        output(&mark_builtin(&ctx.app.settings(), updated))
    }
    .boxed()
}

// The row and the data dir go in one task: a dropped request must not leave files that no row
// names and nothing reclaims. The Profile's backups stay, listed under Backups, and can be restored.
fn delete(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        tokio::spawn(events::keep_actor(async move {
        let input: GetInput = parse(input)?;
        let paths = ctx.app.paths.clone();
        let builtin = ctx.app.settings().oneshot.profile.clone();

        let (deleted, data_dir) = ctx
            .app
            .store
            .run(move |c| {
                let found = profile::find(c, &input.profile)?;
                if found.name == builtin {
                    return Err(Error::InvalidArgument {
                        field: "profile".to_string(),
                        message: format!(
                            "`{builtin}` is the one-shot profile; point oneshot.profile at another profile first"
                        ),
                    });
                }
                if instance::active_for_profile(c, &found.id)?.is_some() {
                    return Err(Error::Conflict {
                        reason: ConflictReason::ProfileRunning,
                        detail: None
                    });
                }

                let data_dir = removable_data_dir(&paths, &found)?;
                profile::delete(c, &found.id)?;

                Ok((with_capabilities(&build::all(c)?, found), data_dir))
            })
            .await?;

        ctx.app
            .events
            .publish(EventData::ProfileDeleted {
                profile_id: deleted.id,
                name: deleted.name.clone(),
            })
            .await?;

        if let Some(dir) = data_dir {
            let removed = dir.clone();
            let result = tokio::task::spawn_blocking(move || fs::remove_dir_all(removed))
                .await
                .map_err(internal)?;
            if let Err(err) = result {
                tracing::error!(profile_id = %deleted.id, path = ?dir, error = format!("{err:#}"), "removing the profile data dir failed");

                return Err(internal(err));
            }
        }

        output(&deleted)
        }))
        .await
        .map_err(internal)?
    }
    .boxed()
}

fn reapply_template(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: ReapplyInput = parse(input)?;
        let dry_run = input.dry_run.unwrap_or(false);

        let (found, diff) = ctx
            .app
            .store
            .run(move |c| {
                let mut found = profile::find(c, &input.profile)?;
                if instance::active_for_profile(c, &found.id)?.is_some() {
                    return Err(Error::Conflict {
                        reason: ConflictReason::ProfileRunning,
                        detail: None,
                    });
                }

                let gone = Error::Conflict {
                    reason: ConflictReason::TemplateGone,
                    detail: None,
                };
                let template = match &found.template_id {
                    Some(template_id) => match template::get(c, template_id) {
                        Err(Error::NotFound { .. }) => return Err(gone),
                        other => other?,
                    },
                    None => return Err(gone),
                };

                let mut changes = Vec::new();
                diff_fields(
                    String::new(),
                    &serde_json::to_value(&found.settings).map_err(internal)?,
                    &serde_json::to_value(&template.settings).map_err(internal)?,
                    &mut changes,
                );
                let had: BTreeSet<ExtensionId> = profile_extension::get(c, &found.id)?
                    .iter()
                    .map(|entry| entry.extension_id)
                    .collect();
                let wants: BTreeSet<ExtensionId> = template
                    .extensions
                    .iter()
                    .map(|entry| entry.extension_id)
                    .collect();
                let diff = ReapplyOutput {
                    changes,
                    extensions: ExtensionChanges {
                        added: wants.difference(&had).copied().collect(),
                        removed: had.difference(&wants).copied().collect(),
                    },
                };

                if !dry_run {
                    template.settings.validate()?;
                    companion::check_settings(c, &template.settings)?;
                    proxies::ensure_exists(c, template.settings.proxy.as_ref())?;
                    extensions::check_list(c, &template.extensions, &template.settings)?;

                    found.settings = template.settings;
                    found.template_name = Some(template.name);
                    found.updated_at = now_ms();
                    profile::update(c, &found)?;
                    profile_extension::set(c, &found.id, &template.extensions)?;
                }

                Ok((found, diff))
            })
            .await?;

        if !dry_run {
            ctx.app
                .events
                .publish(EventData::ProfileUpdated {
                    profile_id: found.id,
                    name: found.name,
                })
                .await?;
        }

        output(&diff)
    }
    .boxed()
}

// Objects are walked key by key so a change reads as `idle.tab_min`, not the whole `idle` object.
fn diff_fields(field: String, from: &Value, to: &Value, changes: &mut Vec<FieldChange>) {
    if let (Value::Object(from), Value::Object(to)) = (from, to) {
        let keys: BTreeSet<&String> = from.keys().chain(to.keys()).collect();
        for key in keys {
            let path = if field.is_empty() {
                key.clone()
            } else {
                format!("{field}.{key}")
            };
            diff_fields(
                path,
                from.get(key).unwrap_or(&Value::Null),
                to.get(key).unwrap_or(&Value::Null),
                changes,
            );
        }

        return;
    }

    if from != to {
        changes.push(FieldChange {
            field,
            from: from.clone(),
            to: to.clone(),
        });
    }
}
