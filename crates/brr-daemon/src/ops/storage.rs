use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration, time::Instant};

use brr_core::{
    ArtifactKind, BuildId, BuildSource, Empty, Error, ExtensionId, Http, Method, OperationSpec,
    ProfileId, ProfileKind, Secret, SecretStatus, Toolset,
    model::BackendKind,
    schema,
    settings::{STORAGE_SECRET, StorageSettings},
};
use brr_storage::{Backend, config};
use brr_store::{artifact, build, retention};
use futures::{FutureExt, StreamExt, future::BoxFuture, stream};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time;

use super::{OpRegistration, output, parse};
use crate::{
    app::{App, Ctx},
    services::{
        profiles,
        quota::{self, dir_size},
        storage::open_backend,
    },
};

const DIR_SIZE_CONCURRENCY: usize = 4;
const TEST_TIMEOUT: Duration = Duration::from_secs(15);
const CREDENTIAL_REJECTIONS: [&str; 2] = ["signaturedoesnotmatch", "invalidaccesskeyid"];

static USAGE: OperationSpec = OperationSpec {
    name: "storage.usage",
    group: "storage",
    toolset: Toolset::Storage,
    summary: "Disk use per Profile, Build, extension and Artifact kind, and the free space",
    http: Http {
        method: Method::Get,
        path: "/v1/storage/usage",
    },
    cli: &["storage", "usage"],
    cli_positional: &[],
    mcp_tool: Some("storage_usage"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<UsageOutput>,
};

static BACKEND_GET: OperationSpec = OperationSpec {
    name: "storage.backend_get",
    group: "storage",
    toolset: Toolset::Storage,
    summary: "Show the Artifact storage backend; the secret shows only as set and a hint",
    http: Http {
        method: Method::Get,
        path: "/v1/storage/backend",
    },
    cli: &["storage", "backend", "get"],
    cli_positional: &[],
    mcp_tool: Some("storage_backend_get"),
    read_only: true,
    capability: None,
    input_schema: schema::<Empty>,
    output_schema: schema::<BackendOutput>,
};

static BACKEND_SET: OperationSpec = OperationSpec {
    name: "storage.backend_set",
    group: "storage",
    toolset: Toolset::Storage,
    summary: "Switch the Artifact storage backend after a connection test; existing Artifacts stay on their old backend",
    http: Http {
        method: Method::Put,
        path: "/v1/storage/backend",
    },
    cli: &["storage", "backend", "set"],
    cli_positional: &[],
    mcp_tool: Some("storage_backend_set"),
    read_only: false,
    capability: None,
    input_schema: schema::<BackendInput>,
    output_schema: schema::<BackendOutput>,
};

static BACKEND_TEST: OperationSpec = OperationSpec {
    name: "storage.backend_test",
    group: "storage",
    toolset: Toolset::Storage,
    summary: "Test a backend config by writing, reading and deleting a small object; an omitted secret reuses the stored one",
    http: Http {
        method: Method::Post,
        path: "/v1/storage/backend/test",
    },
    cli: &["storage", "backend", "test"],
    cli_positional: &[],
    mcp_tool: Some("storage_backend_test"),
    read_only: false,
    capability: None,
    input_schema: schema::<BackendInput>,
    output_schema: schema::<TestOutput>,
};

inventory::submit!(OpRegistration {
    spec: &USAGE,
    handler: usage
});
inventory::submit!(OpRegistration {
    spec: &BACKEND_GET,
    handler: backend_get
});
inventory::submit!(OpRegistration {
    spec: &BACKEND_SET,
    handler: backend_set
});
inventory::submit!(OpRegistration {
    spec: &BACKEND_TEST,
    handler: backend_test
});

#[derive(Serialize, JsonSchema)]
struct UsageOutput {
    profiles: Vec<ProfileUsage>,
    builds: Vec<BuildUsage>,
    extensions: Vec<ExtensionUsage>,
    artifacts: Vec<ArtifactUsage>,
    free_bytes: u64,
    floor_bytes: u64,
}

#[derive(Serialize, JsonSchema)]
struct ProfileUsage {
    profile_id: ProfileId,
    name: String,
    bytes: u64,
}

#[derive(Serialize, JsonSchema)]
struct BuildUsage {
    build_id: BuildId,
    label: String,
    bytes: u64,
}

#[derive(Serialize, JsonSchema)]
struct ExtensionUsage {
    extension_id: ExtensionId,
    name: String,
    bytes: u64,
}

#[derive(Serialize, JsonSchema)]
struct ArtifactUsage {
    kind: ArtifactKind,
    count: u32,
    bytes: u64,
}

#[derive(Serialize, JsonSchema)]
struct BackendOutput {
    backend: BackendKind,
    local_root: PathBuf,
    endpoint: Option<String>,
    region: Option<String>,
    bucket: Option<String>,
    access_key_id: Option<String>,
    path_style: bool,
    r2_account_id: Option<String>,
    public_endpoint: Option<String>,
    secret_access_key: Option<SecretStatus>,
    error: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BackendInput {
    backend: BackendKind,
    endpoint: Option<String>,
    region: Option<String>,
    bucket: Option<String>,
    access_key_id: Option<String>,
    path_style: Option<bool>,
    r2_account_id: Option<String>,
    public_endpoint: Option<String>,
    secret_access_key: Option<Secret>,
}

#[derive(Serialize, JsonSchema)]
struct TestOutput {
    ok: bool,
    elapsed_ms: u64,
}

fn usage(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;
        let app = &ctx.app;

        let profiles = profiles::all(&app.store).await?;
        let (builds, extensions, kinds) = app
            .store
            .run(|c| {
                Ok((
                    build::all(c)?,
                    retention::extension_dirs(c)?,
                    artifact::usage_by_kind(c)?,
                ))
            })
            .await?;

        let persistent: Vec<_> = profiles
            .into_iter()
            .filter(|profile| profile.kind == ProfileKind::Persistent)
            .filter_map(|profile| Some((profile.id, profile.name, profile.data_dir?)))
            .collect();
        let sizes = stream::iter(
            persistent
                .iter()
                .map(|(_, _, dir)| dir.clone())
                .collect::<Vec<_>>(),
        )
        .map(dir_size)
        .buffered(DIR_SIZE_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
        let profiles = persistent
            .into_iter()
            .zip(sizes)
            .map(|((profile_id, name, _), bytes)| ProfileUsage {
                profile_id,
                name,
                bytes,
            })
            .collect();

        let builds = builds
            .into_iter()
            .map(|build| BuildUsage {
                build_id: build.id,
                label: format!("{} {}", build.name, build.version),
                bytes: if build.source == BuildSource::System {
                    0
                } else {
                    build.size_bytes
                },
            })
            .collect();

        let sizes = stream::iter(
            extensions
                .iter()
                .map(|(_, _, dir)| dir.clone())
                .collect::<Vec<_>>(),
        )
        .map(dir_size)
        .buffered(DIR_SIZE_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
        let mut by_extension: Vec<ExtensionUsage> = Vec::new();
        for ((extension_id, name, _), bytes) in extensions.into_iter().zip(sizes) {
            match by_extension
                .iter_mut()
                .find(|usage| usage.extension_id == extension_id)
            {
                Some(usage) => usage.bytes += bytes,
                None => by_extension.push(ExtensionUsage {
                    extension_id,
                    name,
                    bytes,
                }),
            }
        }

        output(&UsageOutput {
            profiles,
            builds,
            extensions: by_extension,
            artifacts: kinds
                .into_iter()
                .map(|(kind, count, bytes)| ArtifactUsage { kind, count, bytes })
                .collect(),
            free_bytes: quota::free_bytes(&app.paths.data_dir)?,
            floor_bytes: quota::floor_bytes(app),
        })
    }
    .boxed()
}

fn backend_get(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        parse::<Empty>(input)?;

        output(&current(&ctx.app).await?)
    }
    .boxed()
}

fn backend_set(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: BackendInput = parse(input)?;
        let app = &ctx.app;
        let settings = candidate_settings(&input);
        let _changing = app.storage.changing().await;
        probe(app, &settings, input.secret_access_key.as_ref()).await?;

        // Settings first: they can still be refused (a key set by env), and then nothing has changed.
        app.apply_settings(storage_changes(&settings)).await?;
        if let Some(secret) = &input.secret_access_key {
            app.secrets.set(&app.store, STORAGE_SECRET, secret).await?;
        }
        app.storage.reload(app).await?;

        output(&current(app).await?)
    }
    .boxed()
}

fn backend_test(ctx: Ctx, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
    async move {
        let input: BackendInput = parse(input)?;
        let settings = candidate_settings(&input);
        let started = Instant::now();
        probe(&ctx.app, &settings, input.secret_access_key.as_ref()).await?;

        output(&TestOutput {
            ok: true,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }
    .boxed()
}

async fn current(app: &Arc<App>) -> Result<BackendOutput, Error> {
    let settings = app.settings().storage.clone();
    let error = open_backend(&app.store, &app.secrets, &app.paths, &settings)
        .await
        .err()
        .map(|error| error.to_string());
    let secret = app.secrets.status(&app.store, STORAGE_SECRET).await?;

    Ok(BackendOutput {
        backend: settings.backend,
        local_root: app.paths.storage_dir(),
        endpoint: settings.endpoint,
        region: settings.region,
        bucket: settings.bucket,
        access_key_id: settings.access_key_id,
        path_style: settings.path_style,
        r2_account_id: settings.r2_account_id,
        public_endpoint: settings.public_endpoint,
        secret_access_key: Some(secret),
        error,
    })
}

fn candidate_settings(input: &BackendInput) -> StorageSettings {
    StorageSettings {
        backend: input.backend,
        endpoint: input.endpoint.clone(),
        region: input.region.clone(),
        bucket: input.bucket.clone(),
        access_key_id: input.access_key_id.clone(),
        path_style: input.path_style.unwrap_or(false),
        r2_account_id: input.r2_account_id.clone(),
        public_endpoint: input.public_endpoint.clone(),
    }
}

async fn probe(
    app: &Arc<App>,
    settings: &StorageSettings,
    given: Option<&Secret>,
) -> Result<(), Error> {
    let secret = match given {
        Some(secret) => Some(Secret::new(secret.expose())),
        None => app.secrets.get(&app.store, STORAGE_SECRET).await?,
    };
    let candidate = Backend::probe(config(settings, secret, &app.paths.storage_dir())?)?;
    let target = settings
        .endpoint
        .clone()
        .or_else(|| {
            settings
                .r2_account_id
                .as_deref()
                .map(brr_storage::r2_endpoint)
        })
        .unwrap_or_else(|| "the local storage directory".to_owned());

    match time::timeout(TEST_TIMEOUT, candidate.test()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(Error::StorageUnavailable { message })) if credentials_rejected(&message) => {
            Err(Error::StorageUnavailable {
                message: format!(
                    "Connected to {target}, but the access key or secret was rejected"
                ),
            })
        }
        Ok(Err(Error::StorageUnavailable { message })) => Err(Error::StorageUnavailable {
            message: format!("Couldn't reach {target}: {}", short_cause(&message)),
        }),
        Ok(Err(other)) => Err(other),
        Err(_) => Err(Error::StorageUnavailable {
            message: format!("Couldn't reach {target}: timed out"),
        }),
    }
}

// The server answered, so this is not a reachability failure.
fn credentials_rejected(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();

    CREDENTIAL_REJECTIONS
        .iter()
        .any(|needle| lower.contains(needle))
}

fn short_cause(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    let known = [
        ("connection refused", "connection refused"),
        ("dns error", "host not found"),
        ("error sending request", "no response from the host"),
        ("timed out", "timed out"),
        (
            "invalidipv4address",
            "bucket addressing failed; turn on path-style requests",
        ),
        (
            "invalidipaddress",
            "bucket addressing failed; turn on path-style requests",
        ),
        (
            "invalidbucketname",
            "bucket addressing failed; turn on path-style requests",
        ),
        ("nosuchbucket", "bucket not found"),
        ("accessdenied", "access denied"),
    ];
    if let Some((_, short)) = known.iter().find(|(needle, _)| lower.contains(needle)) {
        return (*short).to_owned();
    }

    // The tail of the chain is the root cause; the head repeats the probe key and retry settings.
    let tail = message.rsplit(": ").next().unwrap_or(message);
    tail.chars().take(120).collect()
}

fn storage_changes(settings: &StorageSettings) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("storage.backend".to_owned(), json!(settings.backend)),
        ("storage.endpoint".to_owned(), json!(settings.endpoint)),
        ("storage.region".to_owned(), json!(settings.region)),
        ("storage.bucket".to_owned(), json!(settings.bucket)),
        (
            "storage.access_key_id".to_owned(),
            json!(settings.access_key_id),
        ),
        ("storage.path_style".to_owned(), json!(settings.path_style)),
        (
            "storage.r2_account_id".to_owned(),
            json!(settings.r2_account_id),
        ),
        (
            "storage.public_endpoint".to_owned(),
            json!(settings.public_endpoint),
        ),
    ])
}

#[cfg(test)]
mod tests;
