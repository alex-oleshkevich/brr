use std::collections::HashMap;

use brr_core::{Error, LoginCheck, LoginCheckId, LoginStatus, ProfileId};
use rusqlite::{Connection, Row, params};

use crate::{Text, col, expect_changed, first_or_not_found, internal, opt_col, query};

const TABLE: &str = "login_check";
const COLUMNS: &str = "id, profile_id, site, url, expect_selector, expect_text, credential_id, auto_relogin, last_status, last_checked_at";

fn decode(row: &Row) -> rusqlite::Result<LoginCheck> {
    Ok(LoginCheck {
        id: col(row, "id")?,
        profile_id: col(row, "profile_id")?,
        site: row.get("site")?,
        url: row.get("url")?,
        expect_selector: row.get("expect_selector")?,
        expect_text: row.get("expect_text")?,
        credential_id: opt_col(row, "credential_id")?,
        auto_relogin: row.get("auto_relogin")?,
        last_status: col(row, "last_status")?,
        last_checked_at: row.get("last_checked_at")?,
    })
}

pub fn insert(c: &Connection, check: &LoginCheck) -> Result<(), Error> {
    c.execute(
        &format!(
            "INSERT INTO login_check ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
        ),
        params![
            Text(&check.id),
            Text(&check.profile_id),
            check.site,
            check.url,
            check.expect_selector,
            check.expect_text,
            check.credential_id.as_ref().map(Text),
            check.auto_relogin,
            Text(&check.last_status),
            check.last_checked_at,
        ],
    )
    .map_err(internal)?;

    Ok(())
}

// Status and checked-at belong to set_status, so an edit never resets them.
pub fn update(c: &Connection, check: &LoginCheck) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE login_check SET site = ?2, url = ?3, expect_selector = ?4, expect_text = ?5, credential_id = ?6,
             auto_relogin = ?7 WHERE id = ?1",
            params![
                Text(&check.id),
                check.site,
                check.url,
                check.expect_selector,
                check.expect_text,
                check.credential_id.as_ref().map(Text),
                check.auto_relogin,
            ],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("login check {}", check.id))
}

pub fn get(c: &Connection, login_check_id: &LoginCheckId) -> Result<LoginCheck, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM login_check WHERE id = ?1"),
        params![Text(login_check_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("login check {login_check_id}"))
}

pub fn for_profile(c: &Connection, profile_id: &ProfileId) -> Result<Vec<LoginCheck>, Error> {
    query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM login_check WHERE profile_id = ?1 ORDER BY site, id"),
        params![Text(profile_id)],
        decode,
    )
}

pub fn delete(c: &Connection, login_check_id: &LoginCheckId) -> Result<(), Error> {
    let changed = c
        .execute(
            "DELETE FROM login_check WHERE id = ?1",
            params![Text(login_check_id)],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("login check {login_check_id}"))
}

pub fn set_status(
    c: &Connection,
    login_check_id: &LoginCheckId,
    status: LoginStatus,
    now: i64,
) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE login_check SET last_status = ?2, last_checked_at = ?3 WHERE id = ?1",
            params![Text(login_check_id), Text(&status), now],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("login check {login_check_id}"))
}

pub fn profile_statuses(c: &Connection) -> Result<HashMap<ProfileId, LoginStatus>, Error> {
    let rows = query(
        c,
        TABLE,
        "SELECT profile_id, last_status FROM login_check",
        [],
        |row| Ok((col(row, "profile_id")?, col(row, "last_status")?)),
    )?;
    let mut statuses: HashMap<ProfileId, LoginStatus> = HashMap::new();

    for (profile_id, status) in rows {
        let merged = match (statuses.get(&profile_id), status) {
            (Some(LoginStatus::Failed), _) | (_, LoginStatus::Failed) => LoginStatus::Failed,
            (Some(LoginStatus::Unknown), _) | (_, LoginStatus::Unknown) => LoginStatus::Unknown,
            _ => LoginStatus::Ok,
        };

        statuses.insert(profile_id, merged);
    }

    Ok(statuses)
}
