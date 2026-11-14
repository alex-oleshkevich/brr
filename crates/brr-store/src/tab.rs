use brr_core::{Error, InstanceId, Tab, TabId};
use rusqlite::{Connection, Row, params};

use crate::{Text, col, expect_changed, first_or_not_found, internal, query};

const TABLE: &str = "tab";
const COLUMNS: &str = "id, instance_id, target_id, state, url, title, opened_by_brr, pinned, last_activity_at, created_at, closed_at";

fn decode(row: &Row) -> rusqlite::Result<Tab> {
    Ok(Tab {
        id: col(row, "id")?,
        instance_id: col(row, "instance_id")?,
        target_id: col(row, "target_id")?,
        state: col(row, "state")?,
        url: row.get("url")?,
        title: row.get("title")?,
        opened_by_brr: row.get("opened_by_brr")?,
        pinned: row.get("pinned")?,
        last_activity_at: row.get("last_activity_at")?,
        created_at: row.get("created_at")?,
        closed_at: row.get("closed_at")?,
        lease: None,
    })
}

pub fn insert(c: &Connection, t: &Tab) -> Result<(), Error> {
    c.execute(
        &format!(
            "INSERT INTO tab ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
        ),
        params![
            Text(&t.id),
            Text(&t.instance_id),
            Text(&t.target_id),
            Text(&t.state),
            t.url,
            t.title,
            t.opened_by_brr,
            t.pinned,
            t.last_activity_at,
            t.created_at,
            t.closed_at,
        ],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn update(c: &Connection, t: &Tab) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE tab SET instance_id = ?2, target_id = ?3, state = ?4, url = ?5, title = ?6, opened_by_brr = ?7,
                 pinned = ?8, last_activity_at = ?9, created_at = ?10, closed_at = ?11
             WHERE id = ?1",
            params![
                Text(&t.id),
                Text(&t.instance_id),
                Text(&t.target_id),
                Text(&t.state),
                t.url,
                t.title,
                t.opened_by_brr,
                t.pinned,
                t.last_activity_at,
                t.created_at,
                t.closed_at,
            ],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("tab {}", t.id))
}

pub fn get(c: &Connection, tab_id: &TabId) -> Result<Tab, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM tab WHERE id = ?1"),
        params![Text(tab_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("tab {tab_id}"))
}

pub fn list(
    c: &Connection,
    instance_id: &InstanceId,
    include_closed: bool,
) -> Result<Vec<Tab>, Error> {
    query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM tab WHERE instance_id = ?1 AND (?2 OR closed_at IS NULL) ORDER BY created_at, id"
        ),
        params![Text(instance_id), include_closed],
        decode,
    )
}

pub fn close_open(c: &Connection, instance_id: &InstanceId, now: i64) -> Result<usize, Error> {
    c.execute(
        "UPDATE tab SET state = 'closed', closed_at = ?2 WHERE instance_id = ?1 AND closed_at IS NULL",
        params![Text(instance_id), now],
    )
    .map_err(internal)
}
