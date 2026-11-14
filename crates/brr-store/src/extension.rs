use brr_core::{
    Error, Extension, ExtensionId, ExtensionSource, ExtensionVersion, Page, compare_versions,
};
use rusqlite::{Connection, Row, params};

use crate::{
    Text, col, expect_changed, first_or_not_found, internal, name_conflict, next_cursor,
    parse_cursor_pair, query,
};

const TABLE: &str = "extension";
const COLUMNS: &str = "id, engine, source, source_ref, name";

fn decode(row: &Row) -> rusqlite::Result<Extension> {
    Ok(Extension {
        id: col(row, "id")?,
        engine: col(row, "engine")?,
        source: col(row, "source")?,
        source_ref: row.get("source_ref")?,
        name: row.get("name")?,
        versions: Vec::new(),
    })
}

fn decode_version(row: &Row) -> rusqlite::Result<ExtensionVersion> {
    Ok(ExtensionVersion {
        version: row.get("version")?,
        path: col(row, "path")?,
        size_bytes: row.get("size_bytes")?,
        fetched_at: row.get("fetched_at")?,
        last_referenced_at: row.get("last_referenced_at")?,
    })
}

fn with_versions(c: &Connection, mut extensions: Vec<Extension>) -> Result<Vec<Extension>, Error> {
    for e in &mut extensions {
        e.versions = query(
            c,
            "extension_version",
            "SELECT version, path, size_bytes, fetched_at, last_referenced_at FROM extension_version WHERE extension_id = ?1",
            params![Text(&e.id)],
            decode_version,
        )?;
        e.versions
            .sort_by(|a, b| compare_versions(&b.version, &a.version));
    }

    Ok(extensions)
}

pub fn insert(c: &Connection, e: &Extension) -> Result<(), Error> {
    c.execute(
        &format!("INSERT INTO extension ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5)"),
        params![
            Text(&e.id),
            Text(&e.engine),
            Text(&e.source),
            e.source_ref,
            e.name
        ],
    )
    .map_err(name_conflict)?;

    Ok(())
}

pub fn set_name(c: &Connection, extension_id: &ExtensionId, name: &str) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE extension SET name = ?2 WHERE id = ?1",
            params![Text(extension_id), name],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("extension {extension_id}"))
}

pub fn get(c: &Connection, extension_id: &ExtensionId) -> Result<Extension, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM extension WHERE id = ?1"),
        params![Text(extension_id)],
        decode,
    )?;

    first_or_not_found(with_versions(c, rows)?, format!("extension {extension_id}"))
}

pub fn find_by_source(
    c: &Connection,
    source: ExtensionSource,
    source_ref: &str,
) -> Result<Option<Extension>, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM extension WHERE source = ?1 AND source_ref = ?2"),
        params![Text(&source), source_ref],
        decode,
    )?;

    Ok(with_versions(c, rows)?.into_iter().next())
}

// Names are not unique, so the cursor is name then id.
pub fn list(c: &Connection, page: &Page) -> Result<(Vec<Extension>, Option<String>), Error> {
    let (after_name, after_id) = page
        .cursor
        .as_deref()
        .map(parse_cursor_pair)
        .transpose()?
        .unwrap_or_default();

    let rows = query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM extension WHERE name > ?1 OR (name = ?1 AND id > ?2) ORDER BY name, id LIMIT ?3"
        ),
        params![after_name, after_id, page.limit],
        decode,
    )?;
    let next = next_cursor(&rows, page, |e| format!("{}\u{1f}{}", e.name, e.id));

    Ok((with_versions(c, rows)?, next))
}

pub fn delete(c: &Connection, extension_id: &ExtensionId) -> Result<(), Error> {
    let changed = c
        .execute(
            "DELETE FROM extension WHERE id = ?1",
            params![Text(extension_id)],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("extension {extension_id}"))
}

pub fn add_version(
    c: &Connection,
    extension_id: &ExtensionId,
    v: &ExtensionVersion,
) -> Result<(), Error> {
    c.execute(
        "INSERT OR REPLACE INTO extension_version (extension_id, version, path, size_bytes, fetched_at, last_referenced_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            Text(extension_id),
            v.version,
            Text(&v.path),
            v.size_bytes,
            v.fetched_at,
            v.last_referenced_at
        ],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn delete_version(
    c: &Connection,
    extension_id: &ExtensionId,
    version: &str,
) -> Result<(), Error> {
    let changed = c
        .execute(
            "DELETE FROM extension_version WHERE extension_id = ?1 AND version = ?2",
            params![Text(extension_id), version],
        )
        .map_err(internal)?;

    expect_changed(
        changed,
        format!("extension {extension_id} version {version}"),
    )
}

pub fn touch_versions(
    c: &Connection,
    refs: &[(ExtensionId, String)],
    now: i64,
) -> Result<(), Error> {
    let mut touch = c
        .prepare_cached(
            "UPDATE extension_version SET last_referenced_at = ?3 WHERE extension_id = ?1 AND version = ?2",
        )
        .map_err(internal)?;

    for (extension_id, version) in refs {
        touch
            .execute(params![Text(extension_id), version, now])
            .map_err(internal)?;
    }

    Ok(())
}
