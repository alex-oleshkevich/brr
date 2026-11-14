use brr_core::{Error, ExtensionId, ProfileExtension, ProfileId};
use rusqlite::{Connection, Row, params};

use crate::{Text, col, internal, query};

const TABLE: &str = "profile_extension";

fn decode(row: &Row) -> rusqlite::Result<ProfileExtension> {
    Ok(ProfileExtension {
        extension_id: col(row, "extension_id")?,
        pinned_version: row.get("pinned_version")?,
        auto_update: row.get("auto_update")?,
    })
}

pub fn get(c: &Connection, profile_id: &ProfileId) -> Result<Vec<ProfileExtension>, Error> {
    query(
        c,
        TABLE,
        "SELECT extension_id, pinned_version, auto_update FROM profile_extension WHERE profile_id = ?1 ORDER BY extension_id",
        params![Text(profile_id)],
        decode,
    )
}

pub fn set(c: &Connection, profile_id: &ProfileId, list: &[ProfileExtension]) -> Result<(), Error> {
    c.execute(
        "DELETE FROM profile_extension WHERE profile_id = ?1",
        params![Text(profile_id)],
    )
    .map_err(internal)?;

    let mut insert = c
        .prepare_cached(
            "INSERT INTO profile_extension (profile_id, extension_id, pinned_version, auto_update) VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(internal)?;

    for e in list {
        insert
            .execute(params![
                Text(profile_id),
                Text(&e.extension_id),
                e.pinned_version,
                e.auto_update
            ])
            .map_err(internal)?;
    }

    Ok(())
}

pub fn profiles_using(
    c: &Connection,
    extension_id: &ExtensionId,
) -> Result<Vec<(ProfileId, String)>, Error> {
    query(
        c,
        "profile",
        "SELECT id, name FROM profile WHERE id IN (SELECT profile_id FROM profile_extension WHERE extension_id = ?1) ORDER BY name",
        params![Text(extension_id)],
        |row| Ok((col(row, "id")?, row.get("name")?)),
    )
}

pub fn all(c: &Connection) -> Result<Vec<(ProfileId, ProfileExtension)>, Error> {
    query(
        c,
        TABLE,
        "SELECT profile_id, extension_id, pinned_version, auto_update FROM profile_extension ORDER BY profile_id, extension_id",
        [],
        |row| Ok((col(row, "profile_id")?, decode(row)?)),
    )
}
