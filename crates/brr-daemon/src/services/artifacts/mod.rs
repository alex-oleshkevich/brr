use std::{net::Ipv4Addr, path::Path, sync::Arc, time::Duration};

use brr_core::{
    Artifact, ArtifactId, ArtifactKind, Error, EventData, FileOut, InstanceId, MS_PER_DAY,
    ProfileId, TabId, capture::ReturnMode, model::BackendKind, now_ms,
};
use brr_storage::Backend;
use brr_store::{artifact, profile};
use bytes::Bytes;
use schemars::JsonSchema;
use serde::Serialize;
use tracing::warn;

use crate::{
    app::App,
    internal,
    ops::invalid,
    services::{host::BYTES_PER_MB, quota},
};

#[cfg(test)]
mod tests;

const MAX_INLINE_BYTES: u64 = 256 * BYTES_PER_MB;
const MAX_URL_TTL: Duration = Duration::from_secs(7 * 86_400);
const MIN_URL_TTL: Duration = Duration::from_secs(1);
const PUBLIC_PATH: &str = "/v1/a/";
const DEFAULT_NAME: &str = "artifact";
const REPLACEMENT: char = '_';

#[derive(Debug, Clone, Default)]
pub struct ArtifactOwner {
    pub profile_id: Option<ProfileId>,
    pub instance_id: Option<InstanceId>,
    pub tab_id: Option<TabId>,
}

enum Source<'a> {
    Bytes(Bytes),
    File(&'a Path),
}

#[derive(Serialize, JsonSchema, Debug)]
pub struct Delivery {
    pub mime: String,
    pub size: u64,
    pub artifact_id: Option<ArtifactId>,
    pub url: Option<String>,
    pub expires_at: Option<i64>,
    pub inline_base64: Option<FileOut>,
}

pub async fn store_bytes(
    app: &Arc<App>,
    kind: ArtifactKind,
    owner: ArtifactOwner,
    filename: &str,
    mime: &str,
    data: Bytes,
) -> Result<Artifact, Error> {
    store(app, kind, owner, filename, mime, Source::Bytes(data)).await
}

// The file is consumed: the local backend renames it into place.
pub async fn store_file(
    app: &Arc<App>,
    kind: ArtifactKind,
    owner: ArtifactOwner,
    filename: &str,
    mime: &str,
    path: &Path,
) -> Result<Artifact, Error> {
    store(app, kind, owner, filename, mime, Source::File(path)).await
}

pub async fn deliver(
    app: &Arc<App>,
    kind: ArtifactKind,
    owner: ArtifactOwner,
    filename: &str,
    mime: &str,
    data: Vec<u8>,
    mode: ReturnMode,
) -> Result<Delivery, Error> {
    let size = data.len() as u64;
    let (store, inline) = wants(app, mode, size);
    if inline && size > MAX_INLINE_BYTES {
        return Err(invalid(
            "return",
            "the result is over 256 MiB; ask for url instead",
        ));
    }

    let mut delivery = Delivery {
        mime: mime.to_owned(),
        size,
        artifact_id: None,
        url: None,
        expires_at: None,
        inline_base64: None,
    };
    if !store {
        delivery.inline_base64 = inline.then_some(FileOut(data.into()));

        return Ok(delivery);
    }

    let data = Bytes::from(data);
    delivery.inline_base64 = inline.then(|| FileOut(data.clone()));
    let stored = store_bytes(app, kind, owner, filename, mime, data).await?;
    let (url, expires_at) = url(app, &stored).await?;
    delivery.artifact_id = Some(stored.id);
    delivery.url = Some(url);
    delivery.expires_at = Some(expires_at);

    Ok(delivery)
}

pub async fn fetch(
    app: &Arc<App>,
    artifact: &Artifact,
    mode: ReturnMode,
) -> Result<Delivery, Error> {
    let (with_url, inline) = wants(app, mode, artifact.size_bytes);
    let mut delivery = Delivery {
        mime: artifact.mime.clone(),
        size: artifact.size_bytes,
        artifact_id: Some(artifact.id),
        url: None,
        expires_at: None,
        inline_base64: None,
    };

    if with_url {
        let (url, expires_at) = url(app, artifact).await?;
        delivery.url = Some(url);
        delivery.expires_at = Some(expires_at);
    }
    if inline {
        let backend = current_backend(app, artifact)?;
        let data = backend
            .get_bytes(&artifact.location, MAX_INLINE_BYTES)
            .await?;
        delivery.inline_base64 = Some(FileOut(data.into()));
    }

    Ok(delivery)
}

pub async fn url(app: &Arc<App>, artifact: &Artifact) -> Result<(String, i64), Error> {
    let backend = current_backend(app, artifact)?;

    let now = now_ms();
    let expires_at = expires_at(app, artifact).await?;
    if backend.kind() == BackendKind::Local {
        let mut bind = app.settings().server.bind;
        if bind.ip().is_unspecified() {
            bind.set_ip(Ipv4Addr::LOCALHOST.into());
        }

        return Ok((
            format!("http://{bind}{PUBLIC_PATH}{}", artifact.public_id),
            expires_at,
        ));
    }

    let remaining = Duration::from_millis(u64::try_from(expires_at - now).unwrap_or(0));
    let ttl = MAX_URL_TTL
        .min(Duration::from_secs(app.settings().artifacts.url_ttl_s))
        .min(remaining)
        .max(MIN_URL_TTL);
    let url = backend
        .presign_get(&artifact.location, ttl)
        .await?
        .ok_or_else(|| internal("an object store gave no presigned URL"))?;

    Ok((
        url,
        now + i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX),
    ))
}

pub async fn delete(app: &Arc<App>, artifact: &Artifact, reason: &str) -> Result<(), Error> {
    // After a backend switch the old store's credentials are gone: the row is dropped and the
    // object stays there, for the user to clear.
    let backend = app.storage.backend();
    if backend.holds(artifact.backend.as_str(), &artifact.location) {
        remove_object(&backend, &artifact.location).await?;
    } else {
        warn!(
            artifact_id = %artifact.id,
            backend = artifact.backend.as_str(),
            location = artifact.location,
            "artifact is on another backend; its object is left in place"
        );
    }

    let artifact_id = artifact.id;
    app.store
        .run(move |c| artifact::delete(c, &artifact_id))
        .await?;
    app.events
        .publish(EventData::ArtifactDeleted {
            artifact_id,
            kind: artifact.kind,
            size: artifact.size_bytes,
            reason: Some(reason.to_owned()),
        })
        .await?;

    Ok(())
}

async fn store(
    app: &Arc<App>,
    kind: ArtifactKind,
    owner: ArtifactOwner,
    filename: &str,
    mime: &str,
    source: Source<'_>,
) -> Result<Artifact, Error> {
    let backend = app.storage.backend();
    if backend.kind() == BackendKind::Local {
        quota::check_disk_floor(app)?;
    }

    let id = ArtifactId::new();
    let location = format!(
        "{}artifacts/{id}/{}",
        backend.key_prefix(),
        key_name(filename)
    );
    let stored = match source {
        Source::Bytes(data) => backend.put_bytes(&location, data).await?,
        Source::File(path) => backend.put_owned_file(&location, path).await?,
    };

    let created = Artifact {
        id,
        // 128 random bits are the only guard of the unauthenticated content route.
        public_id: format!("{:032x}", rand::random::<u128>()),
        kind,
        profile_id: owner.profile_id,
        instance_id: owner.instance_id,
        tab_id: owner.tab_id,
        mime: mime.to_owned(),
        size_bytes: stored.size_bytes,
        backend: backend.kind(),
        filename: key_name(filename),
        location,
        sha256: stored.sha256,
        created_at: now_ms(),
    };
    let row = created.clone();
    app.store.run(move |c| artifact::insert(c, &row)).await?;
    app.events
        .publish(EventData::ArtifactCreated {
            artifact_id: id,
            kind,
            size: created.size_bytes,
        })
        .await?;

    Ok(created)
}

// (store a URL, inline the bytes)
fn wants(app: &App, mode: ReturnMode, size: u64) -> (bool, bool) {
    let small = size <= app.settings().results.inline_max_mb * BYTES_PER_MB;

    match mode {
        ReturnMode::Auto => (!small, small),
        ReturnMode::Inline => (false, true),
        ReturnMode::Url => (true, false),
        ReturnMode::Both => (true, true),
    }
}

// Objects stay readable only while their backend is the current one.
fn current_backend(app: &App, artifact: &Artifact) -> Result<Backend, Error> {
    let backend = app.storage.backend();
    if !backend.holds(artifact.backend.as_str(), &artifact.location) {
        return Err(Error::StorageUnavailable {
            message: format!(
                "artifact is on another {} backend than the current {} one; switch the storage backend back to reach it",
                artifact.backend.as_str(),
                backend.kind().as_str()
            ),
        });
    }

    Ok(backend)
}

// The Profile's override wins over the global age.
async fn expires_at(app: &Arc<App>, artifact: &Artifact) -> Result<i64, Error> {
    let overridden = match artifact.profile_id {
        Some(profile_id) => {
            match app.store.run(move |c| profile::get(c, &profile_id)).await {
                Ok(found) => found.settings.retention.artifacts_max_age_days,
                // Its Artifacts outlive a deleted Profile.
                Err(Error::NotFound { .. }) => None,
                Err(error) => return Err(error),
            }
        }
        None => None,
    };
    let days = overridden.unwrap_or(app.settings().retention.artifacts_max_age_days);

    Ok(artifact.created_at + i64::from(days) * MS_PER_DAY)
}

async fn remove_object(backend: &Backend, location: &str) -> Result<(), Error> {
    backend.delete(location).await?;

    // The local backend leaves the per-Artifact directory behind.
    if let Some(dir) = backend
        .local_path(location)?
        .as_deref()
        .and_then(Path::parent)
    {
        let _ = tokio::fs::remove_dir(dir).await;
    }

    Ok(())
}

// Backend keys allow [A-Za-z0-9._/-] and no `..`; Artifact filenames come from pages.
fn key_name(filename: &str) -> String {
    let name: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                REPLACEMENT
            }
        })
        .collect::<String>()
        .replace("..", "_");

    // Also catches the empty name and `.`, which would be a path segment.
    if name.chars().all(|c| c == '.') {
        DEFAULT_NAME.to_owned()
    } else if let Some(rest) = name.strip_prefix('.') {
        // A leading dot is how storage names its own temp files.
        format!("{REPLACEMENT}{rest}")
    } else {
        name
    }
}
