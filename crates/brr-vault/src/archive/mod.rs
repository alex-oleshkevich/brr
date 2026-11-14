use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Component, Path};
use std::sync::{Mutex, PoisonError};

use age::secrecy::SecretString;
use age::{DecryptError, Decryptor, Encryptor, Identity, Recipient};
use brr_core::{Error, Secret};

#[cfg(test)]
mod tests;

// Both directions derive their key inside age (wrap_output, decrypt) and hold the scrypt memory
// meanwhile; one at a time keeps concurrent exports and imports from multiplying the peak.
static KDF_GATE: Mutex<()> = Mutex::new(());

pub const MANIFEST_NAME: &str = "profile.json";

const DATA_DIR: &str = "data";
const ZSTD_LEVEL: i32 = 3;
const ARCHIVE_MODE: u32 = 0o600;
const COPY_BUF_BYTES: usize = 64 * 1024;
const DIR_MODE: u32 = 0o700;
// scrypt uses r=8, so memory is 1 KiB * 2^log_n: 128 MiB per export. Pinned, because age's
// default times the local CPU and a fast machine would pick a cost a slow one cannot import.
// Unoptimized test builds would spend over a minute per archive at the real cost.
const ARCHIVE_WORK_FACTOR: u8 = if cfg!(test) { 10 } else { 17 };
// One above the export cost, so archives from a build that raises it by one still import (256 MiB).
const MAX_WORK_FACTOR: u8 = ARCHIVE_WORK_FACTOR + 1;
const MAX_ARCHIVE_ENTRIES: u64 = 200_000;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const PASSPHRASE_FIELD: &str = "passphrase";
const FILE_FIELD: &str = "file";

pub fn write_archive(
    data_dir: &Path,
    manifest: &[u8],
    exclude: &[&str],
    passphrase: &Secret,
    out: &Path,
) -> Result<u64, Error> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(ARCHIVE_MODE)
        .open(out)
        .map_err(|e| io_error("create archive", &e))?;
    let mut recipient = age::scrypt::Recipient::new(SecretString::from(passphrase.expose()));
    recipient.set_work_factor(ARCHIVE_WORK_FACTOR);
    let encryptor = Encryptor::with_recipients(std::iter::once(&recipient as &dyn Recipient))
        .map_err(|e| Error::Internal {
            message: format!("cannot start encryption: {e}"),
        })?;
    let age_writer = {
        let _gate = KDF_GATE.lock().unwrap_or_else(PoisonError::into_inner);
        encryptor
            .wrap_output(file)
            .map_err(|e| io_error("start encryption", &e))?
    };
    let zstd = zstd::Encoder::new(age_writer, ZSTD_LEVEL)
        .map_err(|e| io_error("start compression", &e))?;
    let mut tar = tar::Builder::new(zstd);
    // Sparse entries are rejected on import; holes are stored as zeros instead.
    tar.sparse(false);

    let mut header = tar::Header::new_gnu();
    header.set_size(manifest.len() as u64);
    header.set_mode(ARCHIVE_MODE);
    tar.append_data(&mut header, MANIFEST_NAME, manifest)
        .map_err(|e| io_error("write manifest", &e))?;
    append_tree(&mut tar, data_dir, Path::new(DATA_DIR), exclude)?;

    // Each finish flushes its layer; a dropped age writer would leave an undecryptable last chunk.
    let zstd = tar
        .into_inner()
        .map_err(|e| io_error("finish archive", &e))?;
    let age_writer = zstd
        .finish()
        .map_err(|e| io_error("finish compression", &e))?;
    let file = age_writer
        .finish()
        .map_err(|e| io_error("finish encryption", &e))?;
    file.sync_all().map_err(|e| io_error("sync archive", &e))?;
    file.metadata()
        .map(|m| m.len())
        .map_err(|e| io_error("stat archive", &e))
}

pub fn read_archive(
    archive: &Path,
    passphrase: &Secret,
    dest_dir: &Path,
    max_bytes: u64,
) -> Result<Vec<u8>, Error> {
    let file = File::open(archive).map_err(|e| io_error("open archive", &e))?;
    let decryptor = Decryptor::new_buffered(BufReader::new(file)).map_err(decrypt_error)?;
    let mut identity = age::scrypt::Identity::new(SecretString::from(passphrase.expose()));
    identity.set_max_work_factor(MAX_WORK_FACTOR);
    let reader = {
        let _gate = KDF_GATE.lock().unwrap_or_else(PoisonError::into_inner);
        decryptor
            .decrypt(std::iter::once(&identity as &dyn Identity))
            .map_err(decrypt_error)?
    };
    let zstd = zstd::Decoder::new(reader).map_err(|_| not_an_archive())?;
    let mut tar = tar::Archive::new(zstd);
    let mut entries = tar.entries().map_err(|_| not_an_archive())?;

    let mut first = entries
        .next()
        .and_then(Result::ok)
        .filter(|entry| entry.path().is_ok_and(|p| p == Path::new(MANIFEST_NAME)))
        .ok_or_else(not_an_archive)?;
    let mut manifest = Vec::new();
    first
        .by_ref()
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut manifest)
        .map_err(|_| not_an_archive())?;
    if manifest.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(not_an_archive());
    }

    // A small zstd stream can expand without bound, so extraction stops at the caller's budget.
    let mut remaining = max_bytes;
    // Empty files and directories cost almost nothing to compress but each takes an inode.
    for (index, entry) in entries.enumerate() {
        if index as u64 >= MAX_ARCHIVE_ENTRIES {
            return Err(Error::InvalidArgument {
                field: FILE_FIELD.into(),
                message: format!(
                    "archive has more than {MAX_ARCHIVE_ENTRIES} entries; it is not a brr archive"
                ),
            });
        }

        let mut entry = entry.map_err(|_| corrupt_archive())?;
        let kind = entry.header().entry_type();
        let relative = entry
            .path()
            .ok()
            .and_then(|path| path.strip_prefix(DATA_DIR).ok().map(Path::to_path_buf))
            .filter(|path| path.components().all(|c| matches!(c, Component::Normal(_))))
            .filter(|_| kind.is_file() || kind.is_dir())
            .ok_or_else(unsupported_entry)?;
        if relative.as_os_str().is_empty() {
            continue;
        }

        let target = dest_dir.join(&relative);
        if kind.is_dir() {
            make_dir(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            make_dir(parent)?;
        }
        let mode = entry.header().mode().map_err(|_| corrupt_archive())? & 0o777 | ARCHIVE_MODE;
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&target)
            .map_err(|e| io_error("create file", &e))?;
        copy_entry(&mut entry, &mut out, &mut remaining, max_bytes)?;
    }
    Ok(manifest)
}

fn append_tree<W: Write>(
    tar: &mut tar::Builder<W>,
    dir: &Path,
    name: &Path,
    exclude: &[&str],
) -> Result<(), Error> {
    let mut children = fs::read_dir(dir)
        .and_then(|rd| rd.collect::<Result<Vec<_>, _>>())
        .map_err(|e| io_error("read data directory", &e))?;
    children.sort_by_key(fs::DirEntry::file_name);

    for child in children {
        let file_name = child.file_name();
        if exclude.iter().any(|skip| file_name == *skip) {
            continue;
        }
        let kind = child
            .file_type()
            .map_err(|e| io_error("read data directory", &e))?;
        let child_name = name.join(&file_name);
        if kind.is_dir() {
            tar.append_dir(&child_name, child.path())
                .map_err(|e| io_error("archive directory", &e))?;
            append_tree(tar, &child.path(), &child_name, exclude)?;
        } else if kind.is_file() {
            tar.append_path_with_name(child.path(), &child_name)
                .map_err(|e| io_error("archive file", &e))?;
        }
    }
    Ok(())
}

fn make_dir(path: &Path) -> Result<(), Error> {
    DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(path)
        .map_err(|e| io_error("create directory", &e))
}

// Read failures mean a damaged archive; write failures are local I/O problems.
fn copy_entry(
    from: &mut impl Read,
    to: &mut impl Write,
    remaining: &mut u64,
    max_bytes: u64,
) -> Result<(), Error> {
    let mut buf = [0u8; COPY_BUF_BYTES];
    loop {
        let n = from.read(&mut buf).map_err(|_| corrupt_archive())?;
        if n == 0 {
            return Ok(());
        }
        *remaining = remaining
            .checked_sub(n as u64)
            .ok_or_else(|| Error::InvalidArgument {
                field: FILE_FIELD.into(),
                message: format!(
                    "archive unpacks to more than the {max_bytes} bytes free above the disk floor; free up disk space or delete data first"
                ),
            })?;
        to.write_all(&buf[..n])
            .map_err(|e| io_error("extract file", &e))?;
    }
}

fn decrypt_error(error: DecryptError) -> Error {
    match error {
        DecryptError::NoMatchingKeys
        | DecryptError::DecryptionFailed
        | DecryptError::InvalidHeader
        | DecryptError::InvalidMac
        | DecryptError::UnknownFormat => not_an_archive(),
        DecryptError::ExcessiveWork { .. } => Error::InvalidArgument {
            field: PASSPHRASE_FIELD.into(),
            message: "archive demands more key-derivation work than brr accepts".into(),
        },
        other => Error::Internal {
            message: format!("cannot decrypt archive: {other}"),
        },
    }
}

fn not_an_archive() -> Error {
    Error::InvalidArgument {
        field: PASSPHRASE_FIELD.into(),
        message: "wrong passphrase, or the file is not a brr archive".into(),
    }
}

fn corrupt_archive() -> Error {
    Error::InvalidArgument {
        field: FILE_FIELD.into(),
        message: "archive is corrupt or truncated".into(),
    }
}

fn unsupported_entry() -> Error {
    Error::InvalidArgument {
        field: FILE_FIELD.into(),
        message: "archive contains an unsupported entry".into(),
    }
}

fn io_error(what: &str, error: &io::Error) -> Error {
    Error::Internal {
        message: format!("cannot {what}: {error}"),
    }
}
