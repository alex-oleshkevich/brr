use std::{collections::HashMap, fmt};

use brr_core::{
    CredentialId, CredentialSelectors, CredentialSource, Error, LastLogin, Page, ProfileId,
};
use rusqlite::{Connection, Row, params};

use crate::{
    Json, Text, col, expect_changed, first_or_not_found, internal, json_col, next_cursor,
    parse_cursor_pair, query,
};

const TABLE: &str = "credential";
const COLUMNS: &str = "id, site, login_url, username_enc, password_enc, totp_enc, notes_enc, selectors, created_at, updated_at, source, bitwarden_item_id";

#[derive(Clone)]
pub struct CredentialRow {
    pub id: CredentialId,
    pub site: String,
    pub login_url: Option<String>,
    // None only for Bitwarden rows: the username may be a sealed display hint, the password
    // lives in the vault.
    pub username_enc: Option<Vec<u8>>,
    pub password_enc: Option<Vec<u8>>,
    pub totp_enc: Option<Vec<u8>>,
    pub notes_enc: Option<Vec<u8>>,
    pub selectors: CredentialSelectors,
    pub created_at: i64,
    pub updated_at: i64,
    pub source: CredentialSource,
    pub bitwarden_item_id: Option<String>,
}

impl fmt::Debug for CredentialRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialRow")
            .field("id", &self.id)
            .field("site", &self.site)
            .finish_non_exhaustive()
    }
}

fn decode(row: &Row) -> rusqlite::Result<CredentialRow> {
    Ok(CredentialRow {
        id: col(row, "id")?,
        site: row.get("site")?,
        login_url: row.get("login_url")?,
        username_enc: row.get("username_enc")?,
        password_enc: row.get("password_enc")?,
        totp_enc: row.get("totp_enc")?,
        notes_enc: row.get("notes_enc")?,
        selectors: json_col(row, "selectors")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        source: col(row, "source")?,
        bitwarden_item_id: row.get("bitwarden_item_id")?,
    })
}

pub fn insert(c: &Connection, row: &CredentialRow) -> Result<(), Error> {
    c.execute(
        &format!(
            "INSERT INTO credential ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
        ),
        params![
            Text(&row.id),
            row.site,
            row.login_url,
            row.username_enc,
            row.password_enc,
            row.totp_enc,
            row.notes_enc,
            Json(&row.selectors),
            row.created_at,
            row.updated_at,
            Text(&row.source),
            row.bitwarden_item_id,
        ],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn update(c: &Connection, row: &CredentialRow) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE credential SET site = ?2, login_url = ?3, username_enc = ?4, password_enc = ?5, totp_enc = ?6,
             notes_enc = ?7, selectors = ?8, updated_at = ?9, source = ?10, bitwarden_item_id = ?11 WHERE id = ?1",
            params![
                Text(&row.id),
                row.site,
                row.login_url,
                row.username_enc,
                row.password_enc,
                row.totp_enc,
                row.notes_enc,
                Json(&row.selectors),
                row.updated_at,
                Text(&row.source),
                row.bitwarden_item_id,
            ],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("credential {}", row.id))
}

pub fn get(c: &Connection, credential_id: &CredentialId) -> Result<CredentialRow, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM credential WHERE id = ?1"),
        params![Text(credential_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("credential {credential_id}"))
}

pub fn find_by_bitwarden_item(
    c: &Connection,
    item_id: &str,
) -> Result<Option<CredentialRow>, Error> {
    let rows = query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM credential WHERE source = 'bitwarden' AND bitwarden_item_id = ?1 ORDER BY id LIMIT 1"
        ),
        params![item_id],
        decode,
    )?;

    Ok(rows.into_iter().next())
}

pub fn list(c: &Connection, page: &Page) -> Result<(Vec<CredentialRow>, Option<String>), Error> {
    let after = page.cursor.as_deref().map(parse_cursor_pair).transpose()?;
    let rows = query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM credential WHERE ?1 IS NULL OR (site, id) > (?1, ?2) ORDER BY site, id LIMIT ?3"
        ),
        params![
            after.as_ref().map(|(site, _)| site),
            after.as_ref().map(|(_, id)| id),
            page.limit
        ],
        decode,
    )?;
    let next = next_cursor(&rows, page, |r| format!("{}\u{1f}{}", r.site, r.id));

    Ok((rows, next))
}

pub fn all(c: &Connection) -> Result<Vec<CredentialRow>, Error> {
    query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM credential ORDER BY site, id"),
        [],
        decode,
    )
}

pub fn delete(c: &Connection, credential_id: &CredentialId) -> Result<(), Error> {
    let changed = c
        .execute(
            "DELETE FROM credential WHERE id = ?1",
            params![Text(credential_id)],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("credential {credential_id}"))
}

pub fn set_profiles(
    c: &Connection,
    credential_id: &CredentialId,
    profile_ids: &[ProfileId],
) -> Result<(), Error> {
    c.execute(
        "DELETE FROM credential_profile WHERE credential_id = ?1",
        params![Text(credential_id)],
    )
    .map_err(internal)?;

    for profile_id in profile_ids {
        c.execute(
            "INSERT INTO credential_profile (credential_id, profile_id) VALUES (?1, ?2)",
            params![Text(credential_id), Text(profile_id)],
        )
        .map_err(internal)?;
    }

    Ok(())
}

pub fn profiles(c: &Connection, credential_id: &CredentialId) -> Result<Vec<ProfileId>, Error> {
    query(
        c,
        "credential_profile",
        "SELECT profile_id FROM credential_profile WHERE credential_id = ?1 ORDER BY profile_id",
        params![Text(credential_id)],
        |row| col(row, "profile_id"),
    )
}

pub fn is_linked(
    c: &Connection,
    credential_id: &CredentialId,
    profile_id: &ProfileId,
) -> Result<bool, Error> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM credential_profile WHERE credential_id = ?1 AND profile_id = ?2)",
        params![Text(credential_id), Text(profile_id)],
        |row| row.get(0),
    )
    .map_err(internal)
}

// The newest login derived from the event log: it disappears with event retention.
pub fn last_logins(c: &Connection) -> Result<HashMap<CredentialId, LastLogin>, Error> {
    let rows = query(
        c,
        "event",
        "SELECT json_extract(data, '$.credential_id') AS credential_id, json_extract(data, '$.ok') AS ok, MAX(ts) AS at
         FROM event WHERE kind = 'login.performed' GROUP BY 1",
        [],
        |row| {
            Ok((
                col(row, "credential_id")?,
                LastLogin {
                    ok: row.get::<_, i64>("ok")? != 0,
                    at: row.get("at")?,
                },
            ))
        },
    )?;

    Ok(rows.into_iter().collect())
}
