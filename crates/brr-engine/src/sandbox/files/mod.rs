use std::fs;
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use tar::{Archive, Builder, EntryType, Header};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::runtime::Handle;

#[cfg(test)]
mod tests;

// A Profile folder, run folder or download moved between brr and a remote Host; more is refused.
pub const MAX_TRANSFER_BYTES: u64 = 4 * 1024 * 1024 * 1024;
// The last entry of every archive: a tar cut off between two entries reads like a whole one.
pub const COMPLETE_MARKER: &str = ".brr-complete";
// The runner's answer to a put once the archive is unpacked; any other answer is an error text.
pub const PUT_DONE: &str = "done";

const MARKER_MODE: u32 = 0o600;

// The Open payload of a file stream. brr opens Spec first, then on a remote Host one Put per
// folder and Start; the runner opens Profile once the browser exited. A proxy tunnel's payload is
// empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    // Data: the RunnerSpec as JSON.
    Spec,
    // Data: an archive from `pack` to unpack into the folder; answered with PUT_DONE or an error.
    Put(PathBuf),
    // Answered with the file's size as u64 BE, then its bytes.
    Get(PathBuf),
    // The files are in place: start the browser.
    Start,
    // Data: an archive of the Profile folder.
    Profile,
}

// Lexically under one of `dirs`: absolute, without `.` or `..`, so no prefix check is fooled.
pub fn inside(path: &Path, dirs: &[PathBuf]) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        && dirs.iter().any(|dir| path.starts_with(dir))
}

fn relative(path: &Path) -> bool {
    path.components().next().is_some()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn refused(message: String) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, message)
}

fn too_large() -> io::Error {
    refused(format!(
        "the archive is over the {} GiB transfer cap",
        MAX_TRANSFER_BYTES >> 30
    ))
}

// The tar crate is synchronous: it runs on a blocking thread and drives the async stream from
// there. `left` caps the bytes moved either way.
struct Blocking<T> {
    io: T,
    handle: Handle,
    left: u64,
}

impl<T> Blocking<T> {
    fn new(io: T) -> Blocking<T> {
        Blocking {
            io,
            handle: Handle::current(),
            left: MAX_TRANSFER_BYTES,
        }
    }
}

impl<T: AsyncRead + Unpin> Read for Blocking<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(too_large());
        }
        let max = buf
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let read = self.handle.block_on(self.io.read(&mut buf[..max]))?;
        self.left -= read as u64;

        Ok(read)
    }
}

impl<T: AsyncWrite + Unpin> Write for Blocking<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() as u64 > self.left {
            return Err(too_large());
        }
        let written = self.handle.block_on(self.io.write(buf))?;
        self.left -= written as u64;

        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.handle.block_on(self.io.flush())
    }
}

// Writes `dir`'s folders and regular files as a tar ending in COMPLETE_MARKER and returns the
// writer for the caller to close.
pub async fn pack<W>(dir: PathBuf, writer: W) -> io::Result<W>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let blocking = Blocking::new(writer);

    tokio::task::spawn_blocking(move || {
        let mut builder = Builder::new(blocking);
        // Sparse entries (Chromium preallocates some files) are a tar type the unpacker refuses.
        builder.sparse(false);
        append_tree(&mut builder, &dir, Path::new(""))?;

        let mut marker = Header::new_gnu();
        marker.set_entry_type(EntryType::Regular);
        marker.set_size(0);
        marker.set_mode(MARKER_MODE);
        builder.append_data(&mut marker, COMPLETE_MARKER, io::empty())?;

        let mut blocking = builder.into_inner()?;
        blocking.flush()?;

        Ok(blocking.io)
    })
    .await
    .map_err(io::Error::other)?
}

// Links, sockets and pipes stay behind: a link could point anywhere on the other machine, and
// Chromium's Singleton* links name the machine it ran on.
fn append_tree<W: Write>(builder: &mut Builder<W>, dir: &Path, name: &Path) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        let name = name.join(entry.file_name());

        if file_type.is_dir() {
            builder.append_dir(&name, &path)?;
            append_tree(builder, &path, &name)?;
        } else if file_type.is_file() {
            builder.append_path_with_name(&path, &name)?;
        }
    }

    Ok(())
}

// Unpacks an archive from `pack` into `dir`. The other side is not trusted: only folders and
// regular files below `dir` are accepted, and an archive without its marker is an error.
pub async fn unpack<R>(reader: R, dir: PathBuf) -> io::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let blocking = Blocking::new(reader);

    tokio::task::spawn_blocking(move || {
        fs::create_dir_all(&dir)?;
        unpack_entries(blocking, &dir)
    })
    .await
    .map_err(io::Error::other)?
}

fn unpack_entries(reader: impl Read, dir: &Path) -> io::Result<()> {
    let mut archive = Archive::new(reader);
    let mut complete = false;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if complete {
            return Err(refused(format!(
                "{} follows the end of the archive",
                path.display()
            )));
        }
        if !relative(&path) {
            return Err(refused(format!(
                "{} is not a path inside the folder",
                path.display()
            )));
        }

        let kind = entry.header().entry_type();
        if path == Path::new(COMPLETE_MARKER) && kind == EntryType::Regular {
            complete = true;
            continue;
        }
        if !matches!(kind, EntryType::Regular | EntryType::Directory) {
            return Err(refused(format!(
                "{} is neither a file nor a folder",
                path.display()
            )));
        }
        entry.unpack_in(dir)?;
    }

    if !complete {
        return Err(io::Error::new(
            ErrorKind::UnexpectedEof,
            "the archive ended before its end marker",
        ));
    }

    Ok(())
}
