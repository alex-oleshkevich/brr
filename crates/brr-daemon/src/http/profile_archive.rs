use std::{path::Path, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path as UrlPath, Query, State},
    http::{
        HeaderMap, HeaderValue,
        header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE},
    },
    response::Response,
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use brr_core::{BackupId, Error, Secret};
use brr_store::profile;
use futures::StreamExt;
use serde::Deserialize;
use tokio::{fs::File, io::AsyncWriteExt};

use super::ops::{ApiError, with_logged_client};
use crate::{
    app::App,
    internal,
    services::backups::{self, INLINE_ARCHIVE_MAX, Imported, Scratch},
    wire::{ARCHIVE_MIME, PASSPHRASE_HEADER, file_chunks},
};

const UPLOAD_MODE: u32 = 0o600;
const IMPORT_OP: &str = "profile.import";
const EXPORT_OP: &str = "profile.export";

#[derive(Deserialize)]
struct ImportQuery {
    name: Option<String>,
}

// Archives bypass the JSON routes' body limit: the body is streamed to a file, never held in
// memory, and capped here instead. DefaultBodyLimit only applies to buffering extractors.
pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/v1/profiles/import/raw", post(import))
        .route("/v1/profiles/{profile}/export/raw", post(export))
}

// A header cannot carry arbitrary Unicode, so the value is base64 of the UTF-8 passphrase.
fn passphrase_header(headers: &HeaderMap) -> Result<Secret, Error> {
    let invalid = |message: &str| Error::InvalidArgument {
        field: "passphrase".to_string(),
        message: message.to_string(),
    };
    let encoded = headers
        .get(PASSPHRASE_HEADER)
        .ok_or_else(|| invalid("send the passphrase in the X-Brr-Passphrase header"))?;
    let bytes = STANDARD
        .decode(encoded.as_bytes())
        .map_err(|_| invalid("X-Brr-Passphrase must be base64 of the passphrase"))?;
    let text = String::from_utf8(bytes).map_err(|_| invalid("the passphrase must be UTF-8"))?;

    backups::passphrase(&text)
}

async fn receive(body: Body, path: &Path) -> Result<(), Error> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(UPLOAD_MODE)
        .open(path)
        .await
        .map_err(internal)?;
    let mut received = 0u64;
    let mut chunks = body.into_data_stream();

    while let Some(chunk) = chunks.next().await {
        let chunk: Bytes = chunk.map_err(|e| Error::InvalidArgument {
            field: "body".to_string(),
            message: e.to_string(),
        })?;
        received += chunk.len() as u64;
        if received > INLINE_ARCHIVE_MAX {
            return Err(Error::RequestTooLarge {
                limit_bytes: INLINE_ARCHIVE_MAX,
            });
        }
        file.write_all(&chunk).await.map_err(internal)?;
    }

    file.flush().await.map_err(internal)
}

async fn import(
    State(app): State<Arc<App>>,
    Query(query): Query<ImportQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<Imported>, ApiError> {
    let passphrase = passphrase_header(&headers)?;

    let imported = with_logged_client(app, &headers, IMPORT_OP, |ctx| async move {
        let upload = Scratch::new(&ctx.app, &format!("upload-{}", BackupId::new()))?;
        receive(body, upload.path()).await?;

        backups::import_from_file(&ctx.app, upload, passphrase, query.name).await
    })
    .await?;

    Ok(Json(imported))
}

async fn export(
    State(app): State<Arc<App>>,
    UrlPath(profile_ref): UrlPath<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let passphrase = passphrase_header(&headers)?;

    let (found, archive, size_bytes) =
        with_logged_client(app, &headers, EXPORT_OP, |ctx| async move {
            let found = ctx
                .app
                .store
                .run(move |c| profile::find(c, &profile_ref))
                .await?;
            let (archive, size_bytes) =
                backups::export_to_file(&ctx.app, &found, passphrase).await?;

            Ok((found, archive, size_bytes))
        })
        .await?;

    let file = File::open(archive.path()).await.map_err(internal)?;
    // The archive state carries the Scratch, so the file goes when the download ends or drops.
    let chunks = file_chunks(file, archive);

    let mut response = Response::new(Body::from_stream(chunks));
    let header = |value: String| HeaderValue::from_str(&value).map_err(internal);
    let response_headers = response.headers_mut();
    response_headers.insert(CONTENT_TYPE, HeaderValue::from_static(ARCHIVE_MIME));
    response_headers.insert(CONTENT_LENGTH, header(size_bytes.to_string())?);
    response_headers.insert(
        CONTENT_DISPOSITION,
        header(format!(
            "attachment; filename=\"{}\"",
            backups::archive_filename(&found)
        ))?,
    );

    Ok(response)
}
