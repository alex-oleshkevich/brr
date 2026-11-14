use brr_core::{BuildChoice, BuildId, Channel, Error, Page, PoolId, Profile, ProfileId, ProxyId};
use rusqlite::{Connection, Row, params};

use crate::{
    Json, Text, col, expect_changed, first_or_not_found, internal, json_col, name_conflict,
    next_cursor, opt_col, query,
};

const TABLE: &str = "profile";
const COLUMNS: &str = "id, name, kind, template_id, build_id, channel, settings, pinned, notes, data_dir, disk_quota_bytes, backup_schedule, backup_keep, created_at, updated_at";
const TAGS: &str = "(SELECT json_group_array(tag) FROM (SELECT tag FROM profile_tag WHERE profile_id = profile.id ORDER BY tag)) AS tags";

fn decode(row: &Row) -> rusqlite::Result<Profile> {
    Ok(Profile {
        id: col(row, "id")?,
        name: row.get("name")?,
        kind: col(row, "kind")?,
        template_id: opt_col(row, "template_id")?,
        settings: json_col(row, "settings")?,
        pinned: row.get("pinned")?,
        notes: row.get("notes")?,
        tags: json_col(row, "tags")?,
        data_dir: opt_col(row, "data_dir")?,
        disk_quota_bytes: row.get("disk_quota_bytes")?,
        backup_schedule: row.get("backup_schedule")?,
        backup_keep: row.get("backup_keep")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        capabilities: Vec::new(),
        login_status: None,
        builtin: false,
    })
}

// The Build reference lives in the settings JSON; these columns let M3 ask "is this Build referenced" in SQL.
fn build_columns(p: &Profile) -> (Option<String>, Option<String>) {
    match &p.settings.build {
        BuildChoice::BuildId(build_id) => (Some(build_id.to_string()), None),
        BuildChoice::Channel(channel) => (None, Some(channel.to_string())),
    }
}

fn replace_tags(c: &Connection, p: &Profile) -> Result<(), Error> {
    c.execute(
        "DELETE FROM profile_tag WHERE profile_id = ?1",
        params![Text(&p.id)],
    )
    .map_err(internal)?;

    let mut insert = c
        .prepare_cached("INSERT OR IGNORE INTO profile_tag (profile_id, tag) VALUES (?1, ?2)")
        .map_err(internal)?;

    for tag in &p.tags {
        insert
            .execute(params![Text(&p.id), tag])
            .map_err(internal)?;
    }

    Ok(())
}

pub fn insert(c: &Connection, p: &Profile) -> Result<(), Error> {
    let (build_id, channel) = build_columns(p);

    c.execute(
        &format!("INSERT INTO profile ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"),
        params![
            Text(&p.id),
            p.name,
            Text(&p.kind),
            p.template_id.as_ref().map(Text),
            build_id,
            channel,
            Json(&p.settings),
            p.pinned,
            p.notes,
            p.data_dir.as_ref().map(Text),
            p.disk_quota_bytes,
            p.backup_schedule,
            p.backup_keep,
            p.created_at,
            p.updated_at,
        ],
    )
    .map_err(name_conflict)?;

    replace_tags(c, p)
}

pub fn update(c: &Connection, p: &Profile) -> Result<(), Error> {
    let (build_id, channel) = build_columns(p);

    let changed = c
        .execute(
            "UPDATE profile SET name = ?2, kind = ?3, template_id = ?4, build_id = ?5, channel = ?6, settings = ?7,
                 pinned = ?8, notes = ?9, data_dir = ?10, disk_quota_bytes = ?11, backup_schedule = ?12,
                 backup_keep = ?13, created_at = ?14, updated_at = ?15
             WHERE id = ?1",
            params![
                Text(&p.id),
                p.name,
                Text(&p.kind),
                p.template_id.as_ref().map(Text),
                build_id,
                channel,
                Json(&p.settings),
                p.pinned,
                p.notes,
                p.data_dir.as_ref().map(Text),
                p.disk_quota_bytes,
                p.backup_schedule,
                p.backup_keep,
                p.created_at,
                p.updated_at,
            ],
        )
        .map_err(name_conflict)?;
    expect_changed(changed, format!("profile {}", p.id))?;

    replace_tags(c, p)
}

pub fn get(c: &Connection, profile_id: &ProfileId) -> Result<Profile, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS}, {TAGS} FROM profile WHERE id = ?1"),
        params![Text(profile_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("profile {profile_id}"))
}

pub fn find(c: &Connection, id_or_name: &str) -> Result<Profile, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS}, {TAGS} FROM profile WHERE id = ?1 OR name = ?1"),
        params![id_or_name],
        decode,
    )?;

    first_or_not_found(rows, format!("profile {id_or_name}"))
}

pub fn list(
    c: &Connection,
    tag: Option<&str>,
    page: &Page,
) -> Result<(Vec<Profile>, Option<String>), Error> {
    let rows = query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS}, {TAGS} FROM profile WHERE name > ?1
               AND (?2 IS NULL OR id IN (SELECT profile_id FROM profile_tag WHERE tag = ?2))
             ORDER BY name LIMIT ?3"
        ),
        params![page.cursor.as_deref().unwrap_or_default(), tag, page.limit],
        decode,
    )?;
    let next = next_cursor(&rows, page, |p| p.name.clone());

    Ok((rows, next))
}

pub fn delete(c: &Connection, profile_id: &ProfileId) -> Result<(), Error> {
    let changed = c
        .execute(
            "DELETE FROM profile WHERE id = ?1",
            params![Text(profile_id)],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("profile {profile_id}"))
}

pub fn pinning_build(
    c: &Connection,
    build_id: &BuildId,
) -> Result<Vec<(ProfileId, String)>, Error> {
    named(c, "build_id = ?1", build_id.to_string())
}

pub fn using_proxy(c: &Connection, proxy_id: &ProxyId) -> Result<Vec<(ProfileId, String)>, Error> {
    named(
        c,
        "json_extract(settings, '$.proxy.proxy_id') = ?1",
        proxy_id.to_string(),
    )
}

pub fn using_pool(c: &Connection, pool_id: &PoolId) -> Result<Vec<(ProfileId, String)>, Error> {
    named(
        c,
        "json_extract(settings, '$.proxy.pool_id') = ?1",
        pool_id.to_string(),
    )
}

pub fn followed_channels(c: &Connection) -> Result<Vec<Channel>, Error> {
    let channels: Vec<String> = query(
        c,
        TABLE,
        "SELECT DISTINCT channel FROM profile WHERE channel IS NOT NULL ORDER BY channel",
        [],
        |row| row.get("channel"),
    )?;

    Ok(channels
        .into_iter()
        .filter_map(|channel| match channel.parse() {
            Ok(channel) => Some(channel),
            Err(e) => {
                tracing::warn!(%channel, error = %e, "skipping unparsable profile channel");
                None
            }
        })
        .collect())
}

fn named(
    c: &Connection,
    condition: &str,
    param: String,
) -> Result<Vec<(ProfileId, String)>, Error> {
    query(
        c,
        TABLE,
        &format!("SELECT id, name FROM profile WHERE {condition} ORDER BY name"),
        params![param],
        |row| Ok((col(row, "id")?, row.get("name")?)),
    )
}
