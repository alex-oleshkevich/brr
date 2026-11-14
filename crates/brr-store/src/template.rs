use brr_core::{Error, Page, ProfileSettings, Template, TemplateId};
use rusqlite::{Connection, Row, params};

use crate::{
    Json, Text, col, expect_changed, first_or_not_found, internal, json_col, name_conflict,
    next_cursor, query,
};

const TABLE: &str = "template";
const COLUMNS: &str = "id, name, builtin, settings, created_at, updated_at, description";

fn decode(row: &Row) -> rusqlite::Result<Template> {
    Ok(Template {
        id: col(row, "id")?,
        name: row.get("name")?,
        builtin: row.get("builtin")?,
        settings: json_col(row, "settings")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        description: row.get("description")?,
    })
}

pub fn insert(c: &Connection, t: &Template) -> Result<(), Error> {
    c.execute(
        &format!("INSERT INTO template ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
        params![
            Text(&t.id),
            t.name,
            t.builtin,
            Json(&t.settings),
            t.created_at,
            t.updated_at,
            t.description
        ],
    )
    .map_err(name_conflict)?;

    Ok(())
}

pub fn update(c: &Connection, t: &Template) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE template SET name = ?2, builtin = ?3, settings = ?4, created_at = ?5, updated_at = ?6 WHERE id = ?1",
            params![
                Text(&t.id),
                t.name,
                t.builtin,
                Json(&t.settings),
                t.created_at,
                t.updated_at
            ],
        )
        .map_err(name_conflict)?;

    expect_changed(changed, format!("template {}", t.id))
}

pub fn get(c: &Connection, template_id: &TemplateId) -> Result<Template, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM template WHERE id = ?1"),
        params![Text(template_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("template {template_id}"))
}

pub fn find(c: &Connection, id_or_name: &str) -> Result<Template, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM template WHERE id = ?1 OR name = ?1"),
        params![id_or_name],
        decode,
    )?;

    first_or_not_found(rows, format!("template {id_or_name}"))
}

pub fn list(c: &Connection, page: &Page) -> Result<(Vec<Template>, Option<String>), Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM template WHERE name > ?1 ORDER BY name LIMIT ?2"),
        params![page.cursor.as_deref().unwrap_or_default(), page.limit],
        decode,
    )?;
    let next = next_cursor(&rows, page, |t| t.name.clone());

    Ok((rows, next))
}

pub fn delete(c: &Connection, template_id: &TemplateId) -> Result<(), Error> {
    let changed = c
        .execute(
            "DELETE FROM template WHERE id = ?1",
            params![Text(template_id)],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("template {template_id}"))
}

// A user template that took a built-in's name is left alone.
pub fn upsert_builtin(
    c: &Connection,
    name: &str,
    description: &str,
    settings: &ProfileSettings,
    now: i64,
) -> Result<(), Error> {
    c.execute(
        &format!(
            "INSERT INTO template ({COLUMNS}) VALUES (?1, ?2, 1, ?3, ?4, ?4, ?5)
             ON CONFLICT(name) DO UPDATE SET settings = excluded.settings, updated_at = excluded.updated_at,
                 description = excluded.description
             WHERE builtin = 1"
        ),
        params![Text(&TemplateId::new()), name, Json(settings), now, description],
    )
    .map_err(internal)?;

    Ok(())
}
