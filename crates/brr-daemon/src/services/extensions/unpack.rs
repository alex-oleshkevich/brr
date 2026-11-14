use std::{
    fs::{self, File},
    io::{self, Cursor, Read},
    path::{Component, Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use brr_core::Error;
use serde_json::Value;
use tar::EntryType;
use zip::ZipArchive;

use super::MAX_EXTENSION_BYTES;
use crate::internal;

const MANIFEST: &str = "manifest.json";
const METADATA_DIR: &str = "_metadata";
const MESSAGE_PREFIX: &str = "__MSG_";
const MESSAGE_SUFFIX: &str = "__";
const BOM: &str = "\u{feff}";
const NAME_MAX_CHARS: usize = 200;
const VERSION_MAX_CHARS: usize = 64;
const MAX_ENTRIES: usize = 10_000;

pub enum Archive<'a> {
    Zip(&'a [u8]),
    Tar(&'a [u8]),
}

pub struct Unpacked {
    pub manifest: Value,
    pub version: String,
    pub name: String,
    pub size_bytes: u64,
}

fn rejected(message: impl Into<String>) -> Error {
    Error::InvalidArgument {
        field: "file".to_string(),
        message: message.into(),
    }
}

fn too_many_entries() -> Error {
    rejected(format!("the archive has more than {MAX_ENTRIES} entries"))
}

// Not a path from the archive: every entry is checked before it becomes one.
fn safe_relative(path: &Path) -> Result<PathBuf, Error> {
    let mut clean = PathBuf::new();

    for part in path.components() {
        match part {
            Component::Normal(name) => clean.push(name),
            Component::CurDir => {}
            _ => {
                return Err(rejected(format!(
                    "the archive has an entry outside its folder: {}",
                    path.display()
                )));
            }
        }
    }

    Ok(clean)
}

struct Budget {
    left: u64,
    used: u64,
}

impl Budget {
    fn write(&mut self, source: &mut impl Read, dest: &Path) -> Result<(), Error> {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(internal)?;
        }
        let mut file = File::create(dest).map_err(internal)?;
        let copied = io::copy(&mut source.take(self.left.saturating_add(1)), &mut file)
            .map_err(|e| rejected(format!("the archive cannot be read: {e}")))?;
        if copied > self.left {
            return Err(rejected(format!(
                "the extension unpacks to more than {MAX_EXTENSION_BYTES} bytes"
            )));
        }
        self.left -= copied;
        self.used += copied;

        Ok(())
    }
}

pub fn unpack(archive: Archive<'_>, dest: &Path, key: Option<&[u8]>) -> Result<Unpacked, Error> {
    fs::create_dir_all(dest).map_err(internal)?;
    let mut budget = Budget {
        left: MAX_EXTENSION_BYTES,
        used: 0,
    };

    match archive {
        Archive::Zip(bytes) => unpack_zip(bytes, dest, &mut budget)?,
        Archive::Tar(bytes) => unpack_tar(bytes, dest, &mut budget)?,
    }

    // Chrome refuses an unpacked extension with a `_`-prefixed entry, and the Web Store ships _metadata.
    match fs::remove_dir_all(dest.join(METADATA_DIR)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(internal(e)),
        _ => {}
    }

    let mut manifest = read_manifest(dest)?;
    if let Some(key) = key
        && manifest.get("key").is_none()
    {
        manifest["key"] = Value::String(STANDARD.encode(key));
        let text = serde_json::to_vec_pretty(&manifest).map_err(internal)?;
        fs::write(dest.join(MANIFEST), text).map_err(internal)?;
    }

    let version = manifest_version(&manifest)?;
    let name = display_name(dest, &manifest);

    Ok(Unpacked {
        manifest,
        version,
        name,
        size_bytes: budget.used,
    })
}

fn unpack_zip(bytes: &[u8], dest: &Path, budget: &mut Budget) -> Result<(), Error> {
    let mut zip = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| rejected(format!("not a readable zip archive: {e}")))?;

    if zip.len() > MAX_ENTRIES {
        return Err(too_many_entries());
    }

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| rejected(format!("the archive cannot be read: {e}")))?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(rejected("the archive has an entry outside its folder"));
        };
        if relative.starts_with(METADATA_DIR) {
            continue;
        }
        if entry.is_dir() {
            fs::create_dir_all(dest.join(&relative)).map_err(internal)?;
        } else {
            budget.write(&mut entry, &dest.join(&relative))?;
        }
    }

    Ok(())
}

fn unpack_tar(bytes: &[u8], dest: &Path, budget: &mut Budget) -> Result<(), Error> {
    let mut tar = tar::Archive::new(Cursor::new(bytes));
    let entries = tar
        .entries()
        .map_err(|e| rejected(format!("not a readable tar archive: {e}")))?;

    for (count, entry) in entries.enumerate() {
        if count >= MAX_ENTRIES {
            return Err(too_many_entries());
        }
        let mut entry = entry.map_err(|e| rejected(format!("the archive cannot be read: {e}")))?;
        let path = entry
            .path()
            .map_err(|e| rejected(format!("the archive has an unreadable path: {e}")))?
            .into_owned();
        let relative = safe_relative(&path)?;
        if relative.as_os_str().is_empty() {
            continue;
        }

        match entry.header().entry_type() {
            EntryType::Directory => fs::create_dir_all(dest.join(&relative)).map_err(internal)?,
            EntryType::Regular | EntryType::Continuous => {
                budget.write(&mut entry, &dest.join(&relative))?;
            }
            _ => {
                return Err(rejected(format!(
                    "the archive has a link or special file: {}",
                    path.display()
                )));
            }
        }
    }

    Ok(())
}

fn read_manifest(dest: &Path) -> Result<Value, Error> {
    let text = fs::read_to_string(dest.join(MANIFEST))
        .map_err(|_| rejected("the extension has no readable manifest.json"))?;

    let manifest: Value = serde_json::from_str(text.trim_start_matches(BOM))
        .map_err(|e| rejected(format!("manifest.json is not valid JSON: {e}")))?;
    if !manifest.is_object() {
        return Err(rejected("manifest.json is not a JSON object"));
    }

    Ok(manifest)
}

fn manifest_version(manifest: &Value) -> Result<String, Error> {
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| rejected("manifest.json has no version"))?;
    // The version becomes a directory name that is later deleted.
    let valid = (1..=VERSION_MAX_CHARS).contains(&version.chars().count())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
        && !version.starts_with('.');
    if !valid {
        return Err(rejected("manifest.json has an unusable version"));
    }

    Ok(version.to_string())
}

fn display_name(dest: &Path, manifest: &Value) -> String {
    let raw = manifest
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("extension");
    let resolved = resolve_message(dest, manifest, raw).unwrap_or_else(|| raw.to_string());

    resolved
        .chars()
        .filter(|c| !c.is_control())
        .take(NAME_MAX_CHARS)
        .collect()
}

fn resolve_message(dest: &Path, manifest: &Value, raw: &str) -> Option<String> {
    let key = raw
        .strip_prefix(MESSAGE_PREFIX)?
        .strip_suffix(MESSAGE_SUFFIX)?;
    let locale = manifest.get("default_locale")?.as_str()?;
    let path = safe_relative(Path::new(locale)).ok()?;
    let text = fs::read_to_string(dest.join("_locales").join(path).join("messages.json")).ok()?;
    let messages: Value = serde_json::from_str(text.trim_start_matches(BOM)).ok()?;
    let entry = messages.get(key).or_else(|| {
        messages
            .as_object()?
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    })?;

    Some(entry.get("message")?.as_str()?.to_string())
}

pub fn is_gecko(manifest: &Value) -> bool {
    ["browser_specific_settings", "applications"]
        .iter()
        .any(|field| manifest.get(field).and_then(|v| v.get("gecko")).is_some())
}
