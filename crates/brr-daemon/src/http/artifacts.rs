use std::sync::Arc;

use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{
        HeaderValue, StatusCode,
        header::{
            CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE, LOCATION,
            X_CONTENT_TYPE_OPTIONS,
        },
    },
    response::{IntoResponse, Response},
    routing::get,
};
use brr_core::{Artifact, ArtifactId, ArtifactKind, Error, model::BackendKind};
use brr_storage::Backend;
use brr_store::artifact;

use super::ops::ApiError;
use crate::{app::App, internal, services::artifacts, wire::file_chunks};

const NOSNIFF: &str = "nosniff";
const SANDBOX: &str = "sandbox";
// Pages choose what a Download contains: these would run script on the daemon's own origin.
const ACTIVE_MIMES: [&str; 5] = [
    "text/html",
    "image/svg+xml",
    "application/xml",
    "text/xml",
    "text/javascript",
];

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/v1/artifacts/{artifact_id}/content", get(by_id))
        .route("/v1/a/{public_id}", get(by_public_id))
}

async fn by_id(
    State(app): State<Arc<App>>,
    Path(artifact_id): Path<ArtifactId>,
) -> Result<Response, ApiError> {
    let found = app
        .store
        .run(move |c| artifact::get(c, &artifact_id))
        .await?;

    Ok(content(&app, &found).await?)
}

// No auth by design: the 128-bit id is the capability, so it is never logged.
async fn by_public_id(
    State(app): State<Arc<App>>,
    Path(public_id): Path<String>,
) -> Result<Response, ApiError> {
    let found = app
        .store
        .run(move |c| artifact::by_public_id(c, &public_id))
        .await?;

    Ok(content(&app, &found).await?)
}

async fn content(app: &Arc<App>, found: &Artifact) -> Result<Response, Error> {
    let backend = app.storage.backend();
    if matches!(backend, Backend::Local(_)) && found.backend == BackendKind::Local {
        return stream(&backend, found).await;
    }

    let (url, _) = artifacts::url(app, found).await?;

    Ok((StatusCode::FOUND, [(LOCATION, url)]).into_response())
}

async fn stream(backend: &Backend, found: &Artifact) -> Result<Response, Error> {
    let path = backend
        .local_path(&found.location)?
        .ok_or_else(|| internal("a local backend has no path"))?;
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| Error::NotFound {
            what: "artifact".to_owned(),
        })?;
    let chunks = file_chunks(file, ());
    let header = |value: String| HeaderValue::from_str(&value).map_err(internal);

    let mut response = Response::new(Body::from_stream(chunks));
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, header(found.mime.clone())?);
    headers.insert(CONTENT_LENGTH, header(found.size_bytes.to_string())?);
    let active =
        found.kind == ArtifactKind::Download && ACTIVE_MIMES.contains(&found.mime.as_str());
    let disposition = if active { "attachment" } else { "inline" };
    headers.insert(
        CONTENT_DISPOSITION,
        header(format!("{disposition}; filename=\"{}\"", found.filename))?,
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static(NOSNIFF));
    headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(SANDBOX));

    Ok(response)
}
