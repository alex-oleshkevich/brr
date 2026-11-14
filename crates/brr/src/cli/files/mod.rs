use std::{
    cell::Cell,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    rc::Rc,
};

use anyhow::{Context, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use brr_daemon::wire::{BYTES_PER_MIB, MAX_INLINE_FILE_BYTES};
use serde_json::Value;

#[cfg(test)]
mod tests;

pub(super) const MIB: u64 = BYTES_PER_MIB;
// base64(48 MiB) is 64 MiB; the daemon's body limit is 66 MiB, which leaves room for the envelope.
pub const MAX_UPLOAD_BYTES: u64 = MAX_INLINE_FILE_BYTES;

// text: the file as a UTF-8 string; otherwise a base64 string, a directory packed as a tar stream.
pub fn read_input(path: &Path, text: bool) -> anyhow::Result<Value> {
    let metadata = fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;

    if text {
        if metadata.is_dir() {
            bail!("{} is a directory; pass a text file", path.display());
        }
        let bytes = read_capped(path, metadata.len())?;
        let text = String::from_utf8(bytes)
            .with_context(|| format!("{} is not valid UTF-8 text", path.display()))?;

        return Ok(Value::String(text));
    }

    let bytes = if metadata.is_dir() {
        pack_dir(path)?
    } else {
        read_capped(path, metadata.len())?
    };

    Ok(Value::String(STANDARD.encode(bytes)))
}

const TAIL_CHUNK: u64 = 64 * 1024;
// The last `lines` lines, read backwards in chunks so a log of any size costs only its tail.
pub fn tail_lines(path: &Path, lines: usize) -> io::Result<Vec<String>> {
    let mut file = File::open(path)?;
    let mut end = file.metadata()?.len();
    let mut tail: Vec<u8> = Vec::new();

    // One newline more than `lines` means the first line kept is whole.
    while end > 0 && tail.iter().filter(|byte| **byte == b'\n').count() <= lines {
        let start = end.saturating_sub(TAIL_CHUNK);
        let mut chunk = vec![0; usize::try_from(end - start).map_err(io::Error::other)?];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut chunk)?;

        chunk.extend_from_slice(&tail);
        tail = chunk;
        end = start;
    }

    let text = String::from_utf8_lossy(&tail);
    let all: Vec<&str> = text.lines().collect();

    Ok(all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|line| (*line).to_string())
        .collect())
}

const NEW_FILE_MODE: u32 = 0o600;
const MODE_BITS: u32 = 0o7777;

// A regular file is replaced through a temp file, so a reader never sees half of it. Devices and
// FIFOs are written in place, since renaming over them would replace the node; stdout aliases go
// through the stdout handle.
pub fn write_output(path: &Path, value: &Value, base64: bool) -> anyhow::Result<u64> {
    let text = value
        .as_str()
        .context("the output file field is not a string")?;
    let bytes = if base64 {
        STANDARD
            .decode(text)
            .context("the output file field is not valid base64")?
    } else {
        text.as_bytes().to_vec()
    };

    let written = match fs::metadata(path) {
        _ if STDOUT_ALIASES.iter().any(|alias| path == Path::new(alias)) => {
            io::stdout().lock().write_all(&bytes)
        }
        Ok(target) if !target.is_file() || path.starts_with("/dev") => write_in_place(path, &bytes),
        // The real file, so a symlink keeps pointing at it; its mode stays as the user set it.
        Ok(target) => fs::canonicalize(path)
            .and_then(|real| replace(&real, &bytes, target.permissions().mode() & MODE_BITS)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            replace(path, &bytes, NEW_FILE_MODE)
        }
        Err(error) => Err(error),
    };
    written.with_context(|| format!("cannot write {}", path.display()))?;

    Ok(bytes.len() as u64)
}

fn write_in_place(path: &Path, bytes: &[u8]) -> io::Result<()> {
    OpenOptions::new().write(true).open(path)?.write_all(bytes)
}

// Through the process's own stdout handle, so a redirected stream keeps its file and offset; opening
// the alias again would start writing at offset 0 or, renamed over, replace the redirect target.
const STDOUT_ALIASES: [&str; 3] = ["/dev/stdout", "/dev/fd/1", "/proc/self/fd/1"];

fn replace(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let mut temp_name = path.file_name().unwrap_or_default().to_os_string();
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp = path.with_file_name(temp_name);

    let written = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&temp)
        .and_then(|mut file| file.write_all(bytes))
        .and_then(|()| fs::rename(&temp, path));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }

    written
}

// `size` is what the file claims; a pipe claims 0, so the read is bounded as well.
fn read_capped(path: &Path, size: u64) -> anyhow::Result<Vec<u8>> {
    if size > MAX_UPLOAD_BYTES {
        bail!(
            "{} is {} MB; the limit is {} MB",
            path.display(),
            size / MIB,
            MAX_UPLOAD_BYTES / MIB
        );
    }

    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_UPLOAD_BYTES + 1).read_to_end(&mut bytes))
        .with_context(|| format!("cannot read {}", path.display()))?;
    if bytes.len() as u64 > MAX_UPLOAD_BYTES {
        bail!(
            "{} is over {} MB; the limit is {} MB",
            path.display(),
            MAX_UPLOAD_BYTES / MIB,
            MAX_UPLOAD_BYTES / MIB
        );
    }

    Ok(bytes)
}

fn pack_dir(dir: &Path) -> anyhow::Result<Vec<u8>> {
    let over_limit = Rc::new(Cell::new(false));
    let mut tar = tar::Builder::new(Capped {
        bytes: Vec::new(),
        over_limit: Rc::clone(&over_limit),
    });
    // A link to ~/.ssh must be stored as a link, never followed into the upload.
    tar.follow_symlinks(false);

    let packed = tar.append_dir_all(".", dir).and_then(|()| tar.finish());
    if over_limit.get() {
        bail!(
            "{} packs to more than {} MB; the limit is {} MB",
            dir.display(),
            MAX_UPLOAD_BYTES / MIB,
            MAX_UPLOAD_BYTES / MIB
        );
    }
    packed.with_context(|| format!("cannot pack {}", dir.display()))?;

    Ok(tar.into_inner()?.bytes)
}

struct Capped {
    bytes: Vec<u8>,
    over_limit: Rc<Cell<bool>>,
}

impl Write for Capped {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if (self.bytes.len() + buf.len()) as u64 > MAX_UPLOAD_BYTES {
            self.over_limit.set(true);

            return Err(io::Error::other("over the upload limit"));
        }
        self.bytes.extend_from_slice(buf);

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
