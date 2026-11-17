use brr_core::{BuildChoice, Error, Page, ProfileSettings, Template, TemplateId};
use rusqlite::{Connection, Row, params};

use crate::{
    Json, Text, col, expect_changed, first_or_not_found, internal, json_col, name_conflict,
    next_cursor, query, template_extension,
};

const TABLE: &str = "template";
const COLUMNS: &str = "id, name, builtin, settings, created_at, updated_at, description";

fn decode(row: &Row) -> rusqlite::Result<Template> {
    Ok(Template {
        id: col(row, "id")?,
        name: row.get("name")?,
        builtin: row.get("builtin")?,
        settings: json_col(row, "settings")?,
        extensions: Vec::new(),
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        description: row.get("description")?,
    })
}

fn with_extensions(c: &Connection, mut templates: Vec<Template>) -> Result<Vec<Template>, Error> {
    for t in &mut templates {
        t.extensions = template_extension::get(c, &t.id)?;
    }

    Ok(templates)
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

    template_extension::set(c, &t.id, &t.extensions)
}

pub fn update(c: &Connection, t: &Template) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE template SET name = ?2, builtin = ?3, settings = ?4, created_at = ?5, updated_at = ?6, description = ?7 WHERE id = ?1",
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

    expect_changed(changed, format!("template {}", t.id))?;

    template_extension::set(c, &t.id, &t.extensions)
}

pub fn get(c: &Connection, template_id: &TemplateId) -> Result<Template, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM template WHERE id = ?1"),
        params![Text(template_id)],
        decode,
    )?;

    first_or_not_found(with_extensions(c, rows)?, format!("template {template_id}"))
}

pub fn find(c: &Connection, id_or_name: &str) -> Result<Template, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM template WHERE id = ?1 OR name = ?1"),
        params![id_or_name],
        decode,
    )?;

    first_or_not_found(with_extensions(c, rows)?, format!("template {id_or_name}"))
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

    Ok((with_extensions(c, rows)?, next))
}

// Built-in templates follow the default channels; counting them would make the last stable
// Build impossible to remove.
pub fn user_build_choices(c: &Connection) -> Result<Vec<(TemplateId, String, BuildChoice)>, Error> {
    query(
        c,
        TABLE,
        "SELECT id, name, json_extract(settings, '$.build') AS build FROM template WHERE builtin = 0 ORDER BY name",
        [],
        |row| Ok((col(row, "id")?, row.get("name")?, json_col(row, "build")?)),
    )
}

// Profiles made from it keep their template_name snapshot; a null id tells clients it is gone.
pub fn delete(c: &Connection, template_id: &TemplateId) -> Result<(), Error> {
    c.execute(
        "UPDATE profile SET template_id = NULL WHERE template_id = ?1",
        params![Text(template_id)],
    )
    .map_err(internal)?;
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
