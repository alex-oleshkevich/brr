use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, ErrorKind, Read, Seek, SeekFrom},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use brr_core::Error;
use tracing::warn;
use zip::ZipArchive;

use crate::internal;

const PERMISSION_BITS: u32 = 0o777;
const WALK_DEPTH: usize = 4;
const AR_MAGIC: &[u8] = b"!<arch>\n";
const AR_HEADER_BYTES: usize = 60;
const AR_NAME_BYTES: usize = 16;
const AR_SIZE_RANGE: std::ops::Range<usize> = 48..58;
const DEB_DATA_MEMBER: &str = "data.tar.xz";
const DATA_SUFFIX: &str = ".data.tar.xz";
const STDERR_LIMIT: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    TarXz,
    Deb,
}

impl ArchiveKind {
    pub fn from_name(name: &str) -> Option<ArchiveKind> {
        if name.ends_with(".zip") || name.ends_with(".xpi") {
            Some(ArchiveKind::Zip)
        } else if name.ends_with(".tar.xz") {
            Some(ArchiveKind::TarXz)
        } else if name.ends_with(".deb") {
            Some(ArchiveKind::Deb)
        } else {
            None
        }
    }

    pub(crate) fn extension(self) -> &'static str {
        match self {
            ArchiveKind::Zip => "zip",
            ArchiveKind::TarXz => "tar.xz",
            ArchiveKind::Deb => "deb",
        }
    }
}

pub fn extract(archive: &Path, kind: ArchiveKind, dest: &Path) -> Result<(), Error> {
    match kind {
        ArchiveKind::Zip => {
            extract_zip(archive, dest).map_err(|err| internal("extracting the zip archive", err))
        }
        ArchiveKind::TarXz => untar(archive, dest),
        ArchiveKind::Deb => extract_deb(archive, dest),
    }
}

// Relative to `root`; None when no file of that name sits within WALK_DEPTH levels.
pub(crate) fn find_executable(root: &Path, name: &str) -> Option<PathBuf> {
    walk(root, Path::new(""), name, 1)
}

fn walk(dir: &Path, relative: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    let mut entries: Vec<_> = fs::read_dir(dir).ok()?.filter_map(Result::ok).collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        // file_type() does not follow symlinks, so a link can lead the walk out of the tree.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = relative.join(entry.file_name());
        if file_type.is_file() && entry.file_name() == OsStr::new(name) {
            return Some(path);
        }
        if file_type.is_dir()
            && depth < WALK_DEPTH
            && let Some(found) = walk(&entry.path(), &path, name, depth + 1)
        {
            return Some(found);
        }
    }

    None
}

fn extract_zip(archive: &Path, out: &Path) -> io::Result<()> {
    let mut zip = ZipArchive::new(File::open(archive)?)?;

    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        // Entry names are hostile: enclosed_name() drops a leading `/` and returns None for `..` that escapes.
        let Some(relative) = entry.enclosed_name() else {
            warn!(
                entry = ?entry.name(),
                "skipping zip entry that escapes the install directory"
            );
            continue;
        };
        let target = out.join(relative);

        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        io::copy(&mut entry, &mut File::create(&target)?)?;
        // Without the exec bits chrome and chrome_crashpad_handler fail with EACCES.
        if let Some(mode) = entry.unix_mode() {
            fs::set_permissions(&target, fs::Permissions::from_mode(mode & PERMISSION_BITS))?;
        }
    }

    Ok(())
}

fn extract_deb(archive: &Path, dest: &Path) -> Result<(), Error> {
    let context = "extracting the deb package";
    let mut deb = File::open(archive).map_err(|err| internal(context, err))?;
    let length = ar_member(&mut deb, DEB_DATA_MEMBER)
        .map_err(|err| internal(context, err))?
        .ok_or_else(|| Error::Internal {
            message: format!("{DEB_DATA_MEMBER} not found in the deb package"),
        })?;

    // A sibling of `dest`, so the data member never lands in the install tree.
    let mut data_path = dest.as_os_str().to_owned();
    data_path.push(DATA_SUFFIX);
    let data_path = PathBuf::from(data_path);
    let mut data = File::create(&data_path).map_err(|err| internal(context, err))?;
    let copied = io::copy(&mut (&mut deb).take(length), &mut data);
    let result = copied
        .map_err(|err| internal(context, err))
        .and_then(|_| untar(&data_path, dest));
    fs::remove_file(&data_path).ok();

    result
}

// Leaves `reader` at the start of the named member's data and returns its length.
fn ar_member(reader: &mut (impl Read + Seek), name: &str) -> io::Result<Option<u64>> {
    let mut magic = [0; AR_MAGIC.len()];
    reader.read_exact(&mut magic)?;
    if magic != AR_MAGIC {
        return Err(io::Error::new(ErrorKind::InvalidData, "not an ar archive"));
    }

    let mut header = [0; AR_HEADER_BYTES];
    loop {
        match reader.read_exact(&mut header) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => return Ok(None),
            Err(err) => return Err(err),
        }
        // Names carry a trailing `/` and space padding.
        let member = String::from_utf8_lossy(&header[..AR_NAME_BYTES]);
        let length: u64 = String::from_utf8_lossy(&header[AR_SIZE_RANGE])
            .trim()
            .parse()
            .map_err(|_| io::Error::new(ErrorKind::InvalidData, "bad ar member size"))?;
        if member.trim().trim_end_matches('/') == name {
            return Ok(Some(length));
        }

        // Members are padded to an even length.
        let skip = i64::try_from(length + length % 2)
            .map_err(|_| io::Error::new(ErrorKind::InvalidData, "bad ar member size"))?;
        reader.seek(SeekFrom::Current(skip))?;
    }
}

pub(crate) fn check_xz() -> Result<(), Error> {
    run("xz", &[OsStr::new("--version")])
}

fn untar(archive: &Path, dest: &Path) -> Result<(), Error> {
    run(
        "tar",
        &[
            OsStr::new("-xJf"),
            archive.as_os_str(),
            OsStr::new("-C"),
            dest.as_os_str(),
            OsStr::new("--no-same-owner"),
        ],
    )
}

fn run(program: &str, args: &[&OsStr]) -> Result<(), Error> {
    let output = match Command::new(program).args(args).output() {
        Ok(output) => output,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let package = if program == "xz" { "xz-utils" } else { program };
            return Err(Error::DependencyMissing {
                name: program.to_owned(),
                help: format!("install {package}"),
            });
        }
        Err(err) => return Err(internal(&format!("running {program}"), err)),
    };
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(internal(
        &format!("{program} failed"),
        stderr
            .chars()
            .filter(|c| !c.is_control())
            .take(STDERR_LIMIT)
            .collect::<String>(),
    ))
}

#[cfg(test)]
mod tests;
