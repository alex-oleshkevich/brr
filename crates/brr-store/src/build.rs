use std::path::Path;

use brr_core::{Build, BuildId, BuildSource, Error, Page};
use rusqlite::{Connection, Row, params};

use crate::{
    Json, Text, col, expect_changed, first_or_not_found, internal, json_col, next_cursor,
    parse_cursor, query,
};

const TABLE: &str = "build";
const COLUMNS: &str = "id, source, engine, name, version, channel, path, state, sha256, size_bytes, capabilities, installed_at, last_referenced_at";
const CURSOR_SEPARATOR: char = ':';

fn decode(row: &Row) -> rusqlite::Result<Build> {
    Ok(Build {
        id: col(row, "id")?,
        source: col(row, "source")?,
        engine: col(row, "engine")?,
        name: row.get("name")?,
        version: row.get("version")?,
        channel: row.get("channel")?,
        path: col(row, "path")?,
        state: col(row, "state")?,
        sha256: row.get("sha256")?,
        size_bytes: row.get("size_bytes")?,
        capabilities: json_col(row, "capabilities")?,
        installed_at: row.get("installed_at")?,
        last_referenced_at: row.get("last_referenced_at")?,
    })
}

pub fn insert(c: &Connection, b: &Build) -> Result<(), Error> {
    c.execute(
        &format!("INSERT INTO build ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"),
        params![
            Text(&b.id),
            Text(&b.source),
            Text(&b.engine),
            b.name,
            b.version,
            b.channel,
            Text(&b.path),
            Text(&b.state),
            b.sha256,
            b.size_bytes,
            Json(&b.capabilities),
            b.installed_at,
            b.last_referenced_at,
        ],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn update(c: &Connection, b: &Build) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE build SET source = ?2, engine = ?3, name = ?4, version = ?5, channel = ?6, path = ?7, state = ?8,
                 sha256 = ?9, size_bytes = ?10, capabilities = ?11, installed_at = ?12, last_referenced_at = ?13
             WHERE id = ?1",
            params![
                Text(&b.id),
                Text(&b.source),
                Text(&b.engine),
                b.name,
                b.version,
                b.channel,
                Text(&b.path),
                Text(&b.state),
                b.sha256,
                b.size_bytes,
                Json(&b.capabilities),
                b.installed_at,
                b.last_referenced_at,
            ],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("build {}", b.id))
}

pub fn get(c: &Connection, build_id: &BuildId) -> Result<Build, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM build WHERE id = ?1"),
        params![Text(build_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("build {build_id}"))
}

pub fn find(c: &Connection, source: BuildSource, version: &str) -> Result<Option<Build>, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM build WHERE source = ?1 AND version = ?2"),
        params![Text(&source), version],
        decode,
    )?;

    Ok(rows.into_iter().next())
}

pub fn find_by_path(c: &Connection, path: &Path) -> Result<Option<Build>, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM build WHERE path = ?1"),
        params![Text(&path)],
        decode,
    )?;

    Ok(rows.into_iter().next())
}

// A NULL installed_at sorts as 0 so registered builds without an install time still page stably.
pub fn list(c: &Connection, page: &Page) -> Result<(Vec<Build>, Option<String>), Error> {
    let (after_installed_at, after_id) = match &page.cursor {
        Some(cursor) => {
            let (installed_at, id) = cursor.split_once(CURSOR_SEPARATOR).unwrap_or_default();
            (parse_cursor::<i64>(installed_at)?, id.to_string())
        }
        None => (i64::MAX, String::new()),
    };

    let rows = query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM build
             WHERE COALESCE(installed_at, 0) < ?1 OR (COALESCE(installed_at, 0) = ?1 AND id > ?2)
             ORDER BY COALESCE(installed_at, 0) DESC, id
             LIMIT ?3"
        ),
        params![after_installed_at, after_id, page.limit],
        decode,
    )?;

    let next = next_cursor(&rows, page, |b| {
        format!("{}{CURSOR_SEPARATOR}{}", b.installed_at.unwrap_or(0), b.id)
    });

    Ok((rows, next))
}

pub fn all(c: &Connection) -> Result<Vec<Build>, Error> {
    query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM build ORDER BY rowid"),
        [],
        decode,
    )
}

pub fn mark_interrupted(c: &Connection) -> Result<usize, Error> {
    c.execute(
        "UPDATE build SET state = 'failed' WHERE state = 'installing'",
        [],
    )
    .map_err(internal)
}

pub fn delete(c: &Connection, build_id: &BuildId) -> Result<(), Error> {
    let changed = c
        .execute("DELETE FROM build WHERE id = ?1", params![Text(build_id)])
        .map_err(internal)?;

    expect_changed(changed, format!("build {build_id}"))
}

pub fn touch_referenced(c: &Connection, build_ids: &[BuildId], now: i64) -> Result<(), Error> {
    let mut touch = c
        .prepare_cached("UPDATE build SET last_referenced_at = ?2 WHERE id = ?1")
        .map_err(internal)?;

    for build_id in build_ids {
        touch
            .execute(params![Text(build_id), now])
            .map_err(internal)?;
    }

    Ok(())
}
