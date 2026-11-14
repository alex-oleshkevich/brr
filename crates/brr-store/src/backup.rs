use brr_core::{Backup, BackupId, Error, ProfileId};
use rusqlite::{Connection, Row, params};

use crate::{Text, col, expect_changed, first_or_not_found, internal, query};

const TABLE: &str = "backup";
const INSERT_COLUMNS: &str =
    "id, profile_id, profile_name, backend, location, size_bytes, created_at";
const COLUMNS: &str = "id, profile_id, profile_name, backend, location, size_bytes, created_at,
    NOT EXISTS (SELECT 1 FROM profile WHERE profile.id = backup.profile_id) AS profile_deleted";

fn decode(row: &Row) -> rusqlite::Result<Backup> {
    Ok(Backup {
        id: col(row, "id")?,
        profile_id: col(row, "profile_id")?,
        profile_name: row.get("profile_name")?,
        profile_deleted: row.get("profile_deleted")?,
        backend: row.get("backend")?,
        location: row.get("location")?,
        size_bytes: row.get("size_bytes")?,
        created_at: row.get("created_at")?,
    })
}

pub fn insert(c: &Connection, b: &Backup) -> Result<(), Error> {
    c.execute(
        &format!("INSERT INTO backup ({INSERT_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
        params![
            Text(&b.id),
            Text(&b.profile_id),
            b.profile_name,
            b.backend,
            b.location,
            b.size_bytes,
            b.created_at
        ],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn get(c: &Connection, backup_id: &BackupId) -> Result<Backup, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM backup WHERE id = ?1"),
        params![Text(backup_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("backup {backup_id}"))
}

pub fn for_profile(c: &Connection, profile_id: &ProfileId) -> Result<Vec<Backup>, Error> {
    query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM backup WHERE profile_id = ?1 ORDER BY created_at DESC, id DESC"
        ),
        params![Text(profile_id)],
        decode,
    )
}

// Includes the backups of deleted Profiles.
pub fn all(c: &Connection) -> Result<Vec<Backup>, Error> {
    query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM backup ORDER BY created_at DESC, id DESC"),
        [],
        decode,
    )
}

pub fn delete(c: &Connection, backup_id: &BackupId) -> Result<(), Error> {
    let changed = c
        .execute("DELETE FROM backup WHERE id = ?1", params![Text(backup_id)])
        .map_err(internal)?;

    expect_changed(changed, format!("backup {backup_id}"))
}
