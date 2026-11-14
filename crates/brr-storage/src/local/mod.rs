use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use brr_core::Error;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt, AsyncWriteExt},
};

use crate::{Stored, hex, replace_file, sha256_hex, unavailable, validate_key};

#[cfg(test)]
mod tests;

const COPY_CHUNK: u64 = 1 << 20;

#[derive(Clone)]
pub struct Local {
    root: Arc<PathBuf>,
}

impl Local {
    pub(crate) fn new(root: PathBuf) -> Local {
        Local {
            root: Arc::new(root),
        }
    }

    pub(crate) fn path(&self, key: &str) -> Result<PathBuf, Error> {
        validate_key(key)?;

        Ok(self.root.join(key))
    }

    pub(crate) async fn put_file(&self, key: &str, source: &Path) -> Result<Stored, Error> {
        let path = self.path(key)?;
        let mut input = File::open(source)
            .await
            .map_err(|err| failed("put", key, err))?;

        replace_file(&path, async |output| {
            let mut hasher = Sha256::new();
            let mut size_bytes = 0;
            let mut chunk = Vec::new();

            loop {
                chunk.clear();
                let read = (&mut input)
                    .take(COPY_CHUNK)
                    .read_to_end(&mut chunk)
                    .await
                    .map_err(|err| failed("put", key, err))?;
                if read == 0 {
                    break;
                }
                hasher.update(&chunk);
                output
                    .write_all(&chunk)
                    .await
                    .map_err(|err| failed("put", key, err))?;
                size_bytes += read as u64;
            }

            Ok(Stored {
                size_bytes,
                sha256: hex(&hasher.finalize()),
            })
        })
        .await
    }

    // Moves `source` into place when it shares a filesystem with the root, so a large file is not
    // written twice; across filesystems it is copied and the source removed.
    pub(crate) async fn put_owned_file(&self, key: &str, source: &Path) -> Result<Stored, Error> {
        let path = self.path(key)?;
        let stored = hash_file(source)
            .await
            .map_err(|err| failed("put", key, err))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|err| failed("put", key, err))?;
        }

        match fs::rename(source, &path).await {
            Ok(()) => Ok(stored),
            Err(err) if err.kind() == ErrorKind::CrossesDevices => {
                let copied = self.put_file(key, source).await?;
                let _ = fs::remove_file(source).await;

                Ok(copied)
            }
            Err(err) => Err(failed("put", key, err)),
        }
    }

    pub(crate) async fn put_bytes(&self, key: &str, data: Bytes) -> Result<Stored, Error> {
        let path = self.path(key)?;

        replace_file(&path, async |output| {
            output
                .write_all(&data)
                .await
                .map_err(|err| failed("put", key, err))
        })
        .await?;

        Ok(Stored {
            size_bytes: data.len() as u64,
            sha256: sha256_hex(&data),
        })
    }

    pub(crate) async fn get_to_file(&self, key: &str, target: &Path) -> Result<u64, Error> {
        let mut input = File::open(self.path(key)?)
            .await
            .map_err(|err| failed("get", key, err))?;

        replace_file(target, async |output| {
            tokio::io::copy(&mut input, output)
                .await
                .map_err(|err| failed("get", key, err))
        })
        .await
    }

    pub(crate) async fn get_bytes(&self, key: &str, max_bytes: u64) -> Result<Vec<u8>, Error> {
        let path = self.path(key)?;
        let size = fs::metadata(&path)
            .await
            .map_err(|err| failed("get", key, err))?
            .len();
        if size > max_bytes {
            return Err(unavailable(
                format!("local get of {key}"),
                format!("object is {size} bytes, over the {max_bytes} byte limit"),
            ));
        }

        fs::read(&path).await.map_err(|err| failed("get", key, err))
    }

    pub(crate) async fn delete(&self, key: &str) -> Result<(), Error> {
        match fs::remove_file(self.path(key)?).await {
            Err(err) if err.kind() != ErrorKind::NotFound => Err(failed("delete", key, err)),
            _ => Ok(()),
        }
    }
}

async fn hash_file(path: &Path) -> std::io::Result<Stored> {
    let mut input = File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut size_bytes = 0;
    let mut chunk = Vec::new();

    loop {
        chunk.clear();
        let read = (&mut input)
            .take(COPY_CHUNK)
            .read_to_end(&mut chunk)
            .await?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk);
        size_bytes += read as u64;
    }

    Ok(Stored {
        size_bytes,
        sha256: hex(&hasher.finalize()),
    })
}

fn failed(op: &str, key: &str, err: std::io::Error) -> Error {
    unavailable(format!("local {op} of {key} failed"), err)
}
