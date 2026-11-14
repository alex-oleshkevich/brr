use brr_core::{Artifact, ArtifactId, ArtifactKind, Error, InstanceId, Page, ProfileId};
use rusqlite::{Connection, Row, params};

use crate::{
    Text, col, expect_changed, first_or_not_found, internal, next_cursor, opt_col, parse_cursor,
    query,
};

const TABLE: &str = "artifact";
const COLUMNS: &str = "id, public_id, kind, profile_id, instance_id, tab_id, mime, size_bytes, backend, location, sha256, created_at";

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub kind: Option<ArtifactKind>,
    pub profile_id: Option<ProfileId>,
    pub instance_id: Option<InstanceId>,
}

fn decode(row: &Row) -> rusqlite::Result<Artifact> {
    let location: String = row.get("location")?;
    let filename = location.rsplit('/').next().unwrap_or_default().to_string();

    Ok(Artifact {
        id: col(row, "id")?,
        public_id: row.get("public_id")?,
        kind: col(row, "kind")?,
        profile_id: opt_col(row, "profile_id")?,
        instance_id: opt_col(row, "instance_id")?,
        tab_id: opt_col(row, "tab_id")?,
        mime: row.get("mime")?,
        size_bytes: row.get("size_bytes")?,
        backend: col(row, "backend")?,
        location,
        sha256: row.get("sha256")?,
        created_at: row.get("created_at")?,
        filename,
    })
}

pub fn insert(c: &Connection, a: &Artifact) -> Result<(), Error> {
    let size_bytes = i64::try_from(a.size_bytes).map_err(internal)?;

    c.execute(
        &format!("INSERT INTO artifact ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"),
        params![
            Text(&a.id),
            a.public_id,
            Text(&a.kind),
            a.profile_id.as_ref().map(Text),
            a.instance_id.as_ref().map(Text),
            a.tab_id.as_ref().map(Text),
            a.mime,
            size_bytes,
            Text(&a.backend),
            a.location,
            a.sha256,
            a.created_at
        ],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn get(c: &Connection, artifact_id: &ArtifactId) -> Result<Artifact, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM artifact WHERE id = ?1"),
        params![Text(artifact_id)],
        decode,
    )?;

    first_or_not_found(rows, "artifact")
}

// The unauthenticated content route calls this: exact match on the full id, never a prefix.
pub fn by_public_id(c: &Connection, public_id: &str) -> Result<Artifact, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM artifact WHERE public_id = ?1"),
        params![public_id],
        decode,
    )?;

    first_or_not_found(rows, "artifact")
}

pub fn list(
    c: &Connection,
    filter: &Filter,
    page: &Page,
) -> Result<(Vec<Artifact>, Option<String>), Error> {
    let cursor = page
        .cursor
        .as_deref()
        .map(parse_cursor::<ArtifactId>)
        .transpose()?;
    let rows = query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM artifact
             WHERE (?1 IS NULL OR kind = ?1) AND (?2 IS NULL OR profile_id = ?2)
               AND (?3 IS NULL OR instance_id = ?3) AND (?4 IS NULL OR id < ?4)
             ORDER BY id DESC LIMIT ?5"
        ),
        params![
            filter.kind.as_ref().map(Text),
            filter.profile_id.as_ref().map(Text),
            filter.instance_id.as_ref().map(Text),
            cursor.as_ref().map(Text),
            page.limit
        ],
        decode,
    )?;
    let next = next_cursor(&rows, page, |a| a.id.to_string());

    Ok((rows, next))
}

pub fn delete(c: &Connection, artifact_id: &ArtifactId) -> Result<(), Error> {
    let changed = c
        .execute(
            "DELETE FROM artifact WHERE id = ?1",
            params![Text(artifact_id)],
        )
        .map_err(internal)?;

    expect_changed(changed, "artifact")
}

// Keyset paging in (created_at, id) order: `after` is the last row of the previous page.
pub fn oldest_page(
    c: &Connection,
    after: Option<(i64, ArtifactId)>,
    limit: u32,
) -> Result<Vec<Artifact>, Error> {
    let (created_at, id) = match &after {
        Some((created_at, id)) => (Some(*created_at), Some(Text(id))),
        None => (None, None),
    };

    query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM artifact
             WHERE ?1 IS NULL OR (created_at, id) > (?1, ?2)
             ORDER BY created_at, id LIMIT ?3"
        ),
        params![created_at, id, limit],
        decode,
    )
}

pub fn usage_by_kind(c: &Connection) -> Result<Vec<(ArtifactKind, u32, u64)>, Error> {
    query(
        c,
        TABLE,
        "SELECT kind, count(*), sum(size_bytes) FROM artifact GROUP BY kind ORDER BY kind",
        [],
        |row| Ok((col(row, "kind")?, row.get(1)?, row.get(2)?)),
    )
}
