use brr_core::{Error, ExtensionId, ProfileExtension, TemplateId};
use rusqlite::{Connection, Row, params};

use crate::{Text, col, internal, query};

const TABLE: &str = "template_extension";

fn decode(row: &Row) -> rusqlite::Result<ProfileExtension> {
    Ok(ProfileExtension {
        extension_id: col(row, "extension_id")?,
        pinned_version: row.get("pinned_version")?,
        auto_update: row.get("auto_update")?,
    })
}

pub fn get(c: &Connection, template_id: &TemplateId) -> Result<Vec<ProfileExtension>, Error> {
    query(
        c,
        TABLE,
        "SELECT extension_id, pinned_version, auto_update FROM template_extension WHERE template_id = ?1 ORDER BY extension_id",
        params![Text(template_id)],
        decode,
    )
}

pub fn set(
    c: &Connection,
    template_id: &TemplateId,
    list: &[ProfileExtension],
) -> Result<(), Error> {
    c.execute(
        "DELETE FROM template_extension WHERE template_id = ?1",
        params![Text(template_id)],
    )
    .map_err(internal)?;

    let mut insert = c
        .prepare_cached(
            "INSERT INTO template_extension (template_id, extension_id, pinned_version, auto_update) VALUES (?1, ?2, ?3, ?4)",
        )
        .map_err(internal)?;

    for e in list {
        insert
            .execute(params![
                Text(template_id),
                Text(&e.extension_id),
                e.pinned_version,
                e.auto_update
            ])
            .map_err(internal)?;
    }

    Ok(())
}

pub fn templates_using(
    c: &Connection,
    extension_id: &ExtensionId,
) -> Result<Vec<TemplateId>, Error> {
    query(
        c,
        TABLE,
        "SELECT template_id FROM template_extension WHERE extension_id = ?1 ORDER BY template_id",
        params![Text(extension_id)],
        |row| col(row, "template_id"),
    )
}

pub fn all(c: &Connection) -> Result<Vec<ProfileExtension>, Error> {
    query(
        c,
        TABLE,
        "SELECT extension_id, pinned_version, auto_update FROM template_extension ORDER BY template_id, extension_id",
        [],
        decode,
    )
}
