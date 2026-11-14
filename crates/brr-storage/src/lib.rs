use std::{
    fmt::{self, Display},
    path::{Path, PathBuf},
    time::Duration,
};

use brr_core::{Error, Secret, model::BackendKind, settings::StorageSettings, sha256_hex};
use bytes::Bytes;
use reqwest::Url;
use tokio::{
    fs::{self, File},
    io::AsyncWriteExt,
};
use ulid::Ulid;

pub use local::Local;
pub use s3::S3;

mod local;
mod s3;

const DEFAULT_REGION: &str = "us-east-1";
const R2_REGION: &str = "auto";
const PROBE_PREFIX: &str = "brr-test";
const PROBE_BYTES: &[u8] = b"brr-probe-16byte";
const KEY_TAG_CHARS: usize = 16;
// No stored key can start with this: Artifact names never lead with a dot.
const TEMP_PREFIX: &str = ".brr-tmp-";
const TEMP_SUFFIX: &str = ".tmp";

pub struct S3Config {
    pub kind: BackendKind,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: Secret,
    pub path_style: bool,
    pub public_endpoint: Option<String>,
}

impl fmt::Debug for S3Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Config")
            .field("kind", &self.kind)
            .field("endpoint", &self.endpoint)
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("access_key_id", &self.access_key_id)
            .field("path_style", &self.path_style)
            .field("public_endpoint", &self.public_endpoint)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum Config {
    Local { root: PathBuf },
    S3(S3Config),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone)]
pub enum Backend {
    Local(Local),
    S3(S3),
}

pub fn config(
    settings: &StorageSettings,
    secret: Option<Secret>,
    local_root: &Path,
) -> Result<Config, Error> {
    let kind = settings.backend;
    if kind == BackendKind::Local {
        return Ok(Config::Local {
            root: local_root.to_path_buf(),
        });
    }

    let bucket = required(&settings.bucket, "bucket")?;
    let access_key_id = required(&settings.access_key_id, "access_key_id")?;
    let secret_access_key = secret.ok_or_else(|| missing("secret_access_key"))?;

    let (endpoint, region, path_style) = if kind == BackendKind::R2 {
        if settings
            .endpoint
            .as_deref()
            .is_some_and(|endpoint| !endpoint.is_empty())
        {
            return Err(Error::InvalidArgument {
                field: "endpoint".into(),
                message: "R2 derives the endpoint from r2_account_id".into(),
            });
        }
        let account_id = required(&settings.r2_account_id, "r2_account_id")?;
        (r2_endpoint(&account_id), R2_REGION.into(), true)
    } else {
        let region = settings
            .region
            .clone()
            .filter(|region| !region.is_empty())
            .unwrap_or_else(|| DEFAULT_REGION.into());
        (
            http_url(required(&settings.endpoint, "endpoint")?, "endpoint")?,
            region,
            settings.path_style,
        )
    };

    Ok(Config::S3(S3Config {
        kind,
        endpoint,
        region,
        bucket,
        access_key_id,
        secret_access_key,
        path_style,
        public_endpoint: settings
            .public_endpoint
            .clone()
            .filter(|endpoint| !endpoint.is_empty())
            .map(|endpoint| http_url(endpoint, "public_endpoint"))
            .transpose()?,
    }))
}

pub fn r2_endpoint(account_id: &str) -> String {
    format!("https://{account_id}.r2.cloudflarestorage.com")
}

pub fn validate_key(key: &str) -> Result<(), Error> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-');
    if key.is_empty() || key.starts_with('/') || key.contains("..") || !key.chars().all(allowed) {
        return Err(Error::InvalidArgument {
            field: "key".into(),
            message: format!("{key:?} must be relative, without `..`, of [A-Za-z0-9._/-]"),
        });
    }

    Ok(())
}

impl Backend {
    pub fn open(config: Config) -> Result<Backend, Error> {
        match config {
            Config::Local { root } => Ok(Backend::Local(Local::new(root))),
            Config::S3(config) => Ok(Backend::S3(S3::new(config)?)),
        }
    }

    // Like `open`, but an S3 backend fails fast instead of retrying: for connection tests.
    pub fn probe(config: Config) -> Result<Backend, Error> {
        match config {
            Config::S3(config) => Ok(Backend::S3(S3::probe(config)?)),
            local => Backend::open(local),
        }
    }

    // Leads the key of every object put on an S3 backend: it names the endpoint and bucket, so a
    // row can tell two backends of one kind apart. Local objects live under one root and need none.
    pub fn key_prefix(&self) -> String {
        match self {
            Backend::Local(_) => String::new(),
            Backend::S3(s3) => format!(
                "{}/",
                sha256_hex(s3.identity().as_bytes())
                    .get(..KEY_TAG_CHARS)
                    .unwrap_or_default()
            ),
        }
    }

    // Whether an object recorded with this backend kind and location was written to this backend.
    pub fn holds(&self, kind: &str, location: &str) -> bool {
        self.kind().as_str() == kind && location.starts_with(&self.key_prefix())
    }

    pub fn kind(&self) -> BackendKind {
        match self {
            Backend::Local(_) => BackendKind::Local,
            Backend::S3(s3) => s3.kind(),
        }
    }

    pub async fn put_file(&self, key: &str, path: &Path) -> Result<Stored, Error> {
        let (backend, key, path) = (self.clone(), key.to_owned(), path.to_owned());

        detached(async move {
            match backend {
                Backend::Local(local) => local.put_file(&key, &path).await,
                Backend::S3(s3) => s3.put_file(&key, &path).await,
            }
        })
        .await
    }

    // Takes the source file: the local backend renames it into place, so the caller must not use it after.
    pub async fn put_owned_file(&self, key: &str, path: &Path) -> Result<Stored, Error> {
        let (backend, key, path) = (self.clone(), key.to_owned(), path.to_owned());

        detached(async move {
            match backend {
                Backend::Local(local) => local.put_owned_file(&key, &path).await,
                Backend::S3(s3) => s3.put_file(&key, &path).await,
            }
        })
        .await
    }

    pub async fn put_bytes(&self, key: &str, data: Bytes) -> Result<Stored, Error> {
        let (backend, key) = (self.clone(), key.to_owned());

        detached(async move {
            match backend {
                Backend::Local(local) => local.put_bytes(&key, data).await,
                Backend::S3(s3) => s3.put_bytes(&key, data).await,
            }
        })
        .await
    }

    pub async fn get_to_file(&self, key: &str, path: &Path) -> Result<u64, Error> {
        let (backend, key, path) = (self.clone(), key.to_owned(), path.to_owned());

        detached(async move {
            match backend {
                Backend::Local(local) => local.get_to_file(&key, &path).await,
                Backend::S3(s3) => s3.get_to_file(&key, &path).await,
            }
        })
        .await
    }

    pub async fn get_bytes(&self, key: &str, max_bytes: u64) -> Result<Vec<u8>, Error> {
        match self {
            Backend::Local(local) => local.get_bytes(key, max_bytes).await,
            Backend::S3(s3) => s3.get_bytes(key, max_bytes).await,
        }
    }

    pub fn local_path(&self, key: &str) -> Result<Option<PathBuf>, Error> {
        match self {
            Backend::Local(local) => local.path(key).map(Some),
            Backend::S3(_) => Ok(None),
        }
    }

    pub async fn delete(&self, key: &str) -> Result<(), Error> {
        match self {
            Backend::Local(local) => local.delete(key).await,
            Backend::S3(s3) => s3.delete(key).await,
        }
    }

    pub async fn presign_get(&self, key: &str, ttl: Duration) -> Result<Option<String>, Error> {
        match self {
            Backend::Local(_) => Ok(None),
            Backend::S3(s3) => s3.presign_get(key, ttl).await.map(Some),
        }
    }

    pub async fn test(&self) -> Result<(), Error> {
        let key = format!("{PROBE_PREFIX}/{}", Ulid::generate());

        self.put_bytes(&key, Bytes::from_static(PROBE_BYTES))
            .await
            .map_err(|err| step("put", err))?;
        let read = self.get_bytes(&key, PROBE_BYTES.len() as u64).await;
        // Best effort: the probe is garbage whichever way the read went.
        let deleted = self.delete(&key).await;
        if let Ok(Some(path)) = self.local_path(&key)
            && let Some(dir) = path.parent()
        {
            let _ = fs::remove_dir(dir).await;
        }

        if read.map_err(|err| step("get", err))? != PROBE_BYTES {
            return Err(unavailable("get failed", "probe came back different"));
        }

        deleted.map_err(|err| step("delete", err))
    }
}

// Work that must survive a cancel runs in its own task, so a dropped caller cannot leave a temp file
// or an unfinished multipart upload behind.
pub async fn detached<T: Send + 'static>(
    work: impl Future<Output = Result<T, Error>> + Send + 'static,
) -> Result<T, Error> {
    tokio::spawn(work)
        .await
        .map_err(|err| unavailable("storage task failed", err))?
}

fn required(value: &Option<String>, name: &str) -> Result<String, Error> {
    match value.as_deref() {
        Some(value) if !value.is_empty() => Ok(value.to_owned()),
        _ => Err(missing(name)),
    }
}

// object_store panics on an endpoint that is not a URL, so it is checked before it gets there.
fn http_url(endpoint: String, field: &str) -> Result<String, Error> {
    match Url::parse(&endpoint) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.has_host() => Ok(endpoint),
        _ => Err(Error::InvalidArgument {
            field: field.into(),
            message: format!("{endpoint:?} must be an absolute http:// or https:// URL"),
        }),
    }
}

fn missing(field: &str) -> Error {
    Error::InvalidArgument {
        field: field.into(),
        message: "required by this storage backend".into(),
    }
}

fn unavailable(what: impl Display, cause: impl Display) -> Error {
    Error::StorageUnavailable {
        message: format!("{what}: {cause}"),
    }
}

fn step(name: &str, err: Error) -> Error {
    match err {
        Error::StorageUnavailable { message } => unavailable(format!("{name} failed"), message),
        other => other,
    }
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

// Removes what a crash left beside stored objects: temp files `replace_file` never renamed.
pub fn clear_temp(root: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };

    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();

            match entry.file_type() {
                Ok(kind) if kind.is_dir() => clear_temp(&path),
                Ok(kind)
                    if kind.is_file()
                        && name.starts_with(TEMP_PREFIX)
                        && name.ends_with(TEMP_SUFFIX) =>
                {
                    usize::from(std::fs::remove_file(&path).is_ok())
                }
                _ => 0,
            }
        })
        .sum()
}

fn temp_beside(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(
        "{TEMP_PREFIX}{name}.{}{TEMP_SUFFIX}",
        Ulid::generate()
    ))
}

// Never lose the only good copy: fill a temp file next to `path`, then rename over it.
async fn replace_file<T>(
    path: &Path,
    fill: impl AsyncFnOnce(&mut File) -> Result<T, Error>,
) -> Result<T, Error> {
    let io = |err: std::io::Error| unavailable(format!("writing {}", path.display()), err);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(io)?;
    }
    let temp = temp_beside(path);
    let mut file = File::create(&temp).await.map_err(io)?;

    let result = async {
        let value = fill(&mut file).await?;
        file.flush().await.map_err(io)?;
        fs::rename(&temp, path).await.map_err(io)?;
        Ok(value)
    }
    .await;

    if result.is_err() {
        let _ = fs::remove_file(&temp).await;
    }

    result
}
