use std::{path::Path, time::Duration};

use brr_core::{Error, model::BackendKind};
use bytes::Bytes;
use futures::StreamExt;
use object_store::signer::Method;
use object_store::{
    ClientOptions, Error as StoreError, ObjectStoreExt, RetryConfig, WriteMultipart,
    aws::{AmazonS3, AmazonS3Builder},
    path::Path as ObjectPath,
    signer::Signer,
};
use sha2::{Digest, Sha256};
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt, AsyncWriteExt},
};

use crate::{S3Config, Stored, hex, replace_file, sha256_hex, unavailable, validate_key};

#[cfg(test)]
mod tests;

const SINGLE_PUT_MAX: u64 = 5 << 20;
const CHUNK: u64 = 8 << 20;
const UPLOAD_CONCURRENCY: usize = 4;
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
// A transfer may run for minutes on a slow link, so only a stalled connection is cut off.
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct S3 {
    kind: BackendKind,
    identity: String,
    store: AmazonS3,
    // SigV4 signs the Host header, so presigned URLs need a client built on the public host.
    signer: AmazonS3,
}

impl S3 {
    pub(crate) fn new(config: S3Config) -> Result<S3, Error> {
        S3::with_policy(config, false)
    }

    // A probe makes one attempt with short timeouts, so a connection test answers in seconds.
    pub(crate) fn probe(config: S3Config) -> Result<S3, Error> {
        S3::with_policy(config, true)
    }

    fn with_policy(config: S3Config, probe: bool) -> Result<S3, Error> {
        let store = build(&config, &config.endpoint, probe)?;
        let signer = match &config.public_endpoint {
            Some(public) => build(&config, public, probe)?,
            None => store.clone(),
        };

        Ok(S3 {
            kind: config.kind,
            identity: format!(
                "{}:{}/{}",
                config.kind.as_str(),
                config.endpoint.trim_end_matches('/'),
                config.bucket
            ),
            store,
            signer,
        })
    }

    pub(crate) fn kind(&self) -> BackendKind {
        self.kind
    }

    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }

    pub(crate) async fn put_file(&self, key: &str, path: &Path) -> Result<Stored, Error> {
        let size = fs::metadata(path)
            .await
            .map_err(|err| self.failed("put", key, err))?
            .len();
        if size <= SINGLE_PUT_MAX {
            let data = fs::read(path)
                .await
                .map_err(|err| self.failed("put", key, err))?;

            return self.put_bytes(key, data.into()).await;
        }

        let object_path = self.object_path(key)?;
        let mut file = File::open(path)
            .await
            .map_err(|err| self.failed("put", key, err))?;
        let upload = self
            .store
            .put_multipart(&object_path)
            .await
            .map_err(|err| self.failed("put", key, err))?;
        let mut write = WriteMultipart::new_with_chunk_size(upload, CHUNK as usize);

        let result = async {
            let mut hasher = Sha256::new();
            let mut size_bytes = 0;
            loop {
                let mut chunk = Vec::with_capacity(CHUNK as usize);
                let read = (&mut file)
                    .take(CHUNK)
                    .read_to_end(&mut chunk)
                    .await
                    .map_err(|err| self.failed("put", key, err))?;
                if read == 0 {
                    break;
                }
                hasher.update(&chunk);
                size_bytes += read as u64;
                write
                    .wait_for_capacity(UPLOAD_CONCURRENCY)
                    .await
                    .map_err(|err| self.failed("put", key, err))?;
                write.put(chunk.into());
            }

            Ok(Stored {
                size_bytes,
                sha256: hex(&hasher.finalize()),
            })
        }
        .await;

        match result {
            Ok(stored) => {
                write
                    .finish()
                    .await
                    .map_err(|err| self.failed("put", key, err))?;

                Ok(stored)
            }
            Err(err) => {
                let _ = write.abort().await;

                Err(err)
            }
        }
    }

    pub(crate) async fn put_bytes(&self, key: &str, data: Bytes) -> Result<Stored, Error> {
        let path = self.object_path(key)?;
        let stored = Stored {
            size_bytes: data.len() as u64,
            sha256: sha256_hex(&data),
        };

        self.store
            .put(&path, data.into())
            .await
            .map_err(|err| self.failed("put", key, err))?;

        Ok(stored)
    }

    pub(crate) async fn get_to_file(&self, key: &str, target: &Path) -> Result<u64, Error> {
        let mut stream = self
            .store
            .get(&self.object_path(key)?)
            .await
            .map_err(|err| self.failed("get", key, err))?
            .into_stream();

        replace_file(target, async |output| {
            let mut size = 0;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|err| self.failed("get", key, err))?;
                output
                    .write_all(&chunk)
                    .await
                    .map_err(|err| self.failed("get", key, err))?;
                size += chunk.len() as u64;
            }

            Ok(size)
        })
        .await
    }

    pub(crate) async fn get_bytes(&self, key: &str, max_bytes: u64) -> Result<Vec<u8>, Error> {
        let result = self
            .store
            .get(&self.object_path(key)?)
            .await
            .map_err(|err| self.failed("get", key, err))?;
        if result.meta.size > max_bytes {
            return Err(unavailable(
                format!("{:?} get of {key}", self.kind),
                format!(
                    "object is {} bytes, over the {max_bytes} byte limit",
                    result.meta.size
                ),
            ));
        }

        let bytes = result
            .bytes()
            .await
            .map_err(|err| self.failed("get", key, err))?;

        Ok(bytes.to_vec())
    }

    pub(crate) async fn delete(&self, key: &str) -> Result<(), Error> {
        match self.store.delete(&self.object_path(key)?).await {
            Err(err) if !matches!(err, StoreError::NotFound { .. }) => {
                Err(self.failed("delete", key, err))
            }
            _ => Ok(()),
        }
    }

    pub(crate) async fn presign_get(&self, key: &str, ttl: Duration) -> Result<String, Error> {
        let url = self
            .signer
            .signed_url(Method::GET, &self.object_path(key)?, ttl)
            .await
            .map_err(|err| self.failed("presign", key, err))?;

        Ok(url.to_string())
    }

    fn object_path(&self, key: &str) -> Result<ObjectPath, Error> {
        validate_key(key)?;

        ObjectPath::parse(key).map_err(|err| self.failed("path", key, err))
    }

    fn failed(&self, op: &str, key: &str, err: impl std::fmt::Display) -> Error {
        unavailable(format!("{:?} {op} of {key} failed", self.kind), err)
    }
}

fn build(config: &S3Config, endpoint: &str, probe: bool) -> Result<AmazonS3, Error> {
    // object_store's virtual-hosted mode uses a custom endpoint as-is, so the bucket goes into the host here.
    let endpoint = match endpoint.split_once("://") {
        Some((scheme, host)) if !config.path_style => {
            format!("{scheme}://{}.{host}", config.bucket)
        }
        _ => endpoint.to_owned(),
    };

    let mut builder = AmazonS3Builder::new()
        .with_endpoint(&endpoint)
        .with_region(&config.region)
        .with_bucket_name(&config.bucket)
        .with_access_key_id(&config.access_key_id)
        .with_secret_access_key(config.secret_access_key.expose())
        .with_virtual_hosted_style_request(!config.path_style);
    // with_client_options replaces the whole set, so it goes before the options that follow it.
    builder = if probe {
        builder
            .with_retry(RetryConfig {
                max_retries: 0,
                retry_timeout: PROBE_TIMEOUT,
                ..RetryConfig::default()
            })
            .with_client_options(
                ClientOptions::new()
                    .with_connect_timeout(PROBE_TIMEOUT)
                    .with_timeout(PROBE_TIMEOUT),
            )
    } else {
        builder.with_client_options(
            ClientOptions::new()
                .with_timeout_disabled()
                .with_connect_timeout(CONNECT_TIMEOUT)
                .with_read_timeout(READ_IDLE_TIMEOUT),
        )
    };

    builder
        .with_allow_http(endpoint.starts_with("http://"))
        .build()
        .map_err(|err| unavailable(format!("{:?} client for {endpoint}", config.kind), err))
}
