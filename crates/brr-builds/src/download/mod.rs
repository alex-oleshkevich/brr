use std::{io::ErrorKind, path::Path};

use brr_core::{Error, InstallPhase};
use reqwest::{
    StatusCode,
    header::{CONTENT_RANGE, RANGE},
};
use sha2::{Digest, Sha256};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

use crate::internal;

const PROGRESS_STEP_BYTES: u64 = 1 << 20;
const HASH_BUFFER_BYTES: usize = 64 << 10;

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub bytes: u64,
    pub total: Option<u64>,
    pub phase: InstallPhase,
}

pub struct Downloaded {
    pub sha256: String,
    pub size_bytes: u64,
}

pub async fn download(
    http: &reqwest::Client,
    url: &str,
    part: &Path,
    expected_sha256: Option<&str>,
    progress: Option<mpsc::Sender<Progress>>,
) -> Result<Downloaded, Error> {
    // Without a checksum the bytes of a part cannot be vouched for, and the URL may now serve a
    // newer file than the one the part began.
    let mut resume = expected_sha256.is_some();

    loop {
        let mut hasher = Sha256::new();
        let mut offset = if resume {
            hash_existing(part, &mut hasher).await?
        } else {
            0
        };

        let mut request = http.get(url);
        if offset > 0 {
            request = request.header(RANGE, format!("bytes={offset}-"));
        }
        let mut response = request
            .send()
            .await
            .map_err(|err| internal(&format!("downloading {url}"), err))?;

        let status = response.status();
        if status == StatusCode::RANGE_NOT_SATISFIABLE && offset > 0 {
            // A part that is already the whole file, e.g. kept after a failed extraction.
            if complete_length(&response) == Some(offset) {
                return verified(url, part, expected_sha256, hex(&hasher.finalize()), offset).await;
            }
            fs::remove_file(part)
                .await
                .map_err(|err| internal("removing the partial download", err))?;
            resume = false;
            continue;
        }
        if status != StatusCode::OK && status != StatusCode::PARTIAL_CONTENT {
            return Err(internal(
                &format!("downloading {url}"),
                format!("server answered {status}"),
            ));
        }

        // A 200 means the server ignored the Range header and sent the whole file.
        let appending = status == StatusCode::PARTIAL_CONTENT;
        if !appending {
            offset = 0;
            hasher = Sha256::new();
        }
        let total = response.content_length().map(|length| length + offset);
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .append(appending)
            .truncate(!appending)
            .open(part)
            .await
            .map_err(|err| internal("opening the partial download", err))?;

        let mut bytes = offset;
        let mut reported = offset;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|err| internal(&format!("downloading {url}"), err))?
        {
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|err| internal("writing the partial download", err))?;
            bytes += chunk.len() as u64;

            if bytes - reported >= PROGRESS_STEP_BYTES {
                reported = bytes;
                send_progress(
                    progress.as_ref(),
                    Progress {
                        bytes,
                        total,
                        phase: InstallPhase::Downloading,
                    },
                )
                .await;
            }
        }
        file.flush()
            .await
            .map_err(|err| internal("writing the partial download", err))?;
        send_progress(
            progress.as_ref(),
            Progress {
                bytes,
                total,
                phase: InstallPhase::Downloading,
            },
        )
        .await;

        return verified(url, part, expected_sha256, hex(&hasher.finalize()), bytes).await;
    }
}

async fn verified(
    url: &str,
    part: &Path,
    expected_sha256: Option<&str>,
    sha256: String,
    size_bytes: u64,
) -> Result<Downloaded, Error> {
    if let Some(expected) = expected_sha256
        && !expected.eq_ignore_ascii_case(&sha256)
    {
        // Keeping the part would make every retry resume the corrupt file.
        fs::remove_file(part)
            .await
            .map_err(|err| internal("removing the corrupt download", err))?;
        return Err(Error::Internal {
            message: format!("checksum mismatch for {url}: expected {expected}, got {sha256}"),
        });
    }

    Ok(Downloaded { sha256, size_bytes })
}

// A 416 answers `Content-Range: bytes */<length>`.
fn complete_length(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(CONTENT_RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes */")?
        .parse()
        .ok()
}

async fn send_progress(progress: Option<&mpsc::Sender<Progress>>, event: Progress) {
    if let Some(progress) = progress {
        // A dropped receiver only means nobody watches the progress.
        progress.send(event).await.ok();
    }
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn hash_existing(part: &Path, hasher: &mut Sha256) -> Result<u64, Error> {
    let mut file = match File::open(part).await {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(internal("opening the partial download", err)),
    };

    let mut buffer = vec![0; HASH_BUFFER_BYTES];
    let mut length = 0;
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|err| internal("reading the partial download", err))?;
        if read == 0 {
            return Ok(length);
        }
        hasher.update(&buffer[..read]);
        length += read as u64;
    }
}

#[cfg(test)]
mod tests;
