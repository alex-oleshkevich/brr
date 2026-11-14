use brr_core::{Client, ClientId, Error, TabId};
use rusqlite::{Connection, Row, params};

use crate::{Text, col, expect_changed, first_or_not_found, internal, opt_col, query};

const TABLE: &str = "client";
const COLUMNS: &str = "id, kind, label, current_tab_id, created_at, ended_at";

fn decode(row: &Row) -> rusqlite::Result<Client> {
    Ok(Client {
        id: col(row, "id")?,
        kind: col(row, "kind")?,
        label: row.get("label")?,
        current_tab_id: opt_col(row, "current_tab_id")?,
        created_at: row.get("created_at")?,
        ended_at: row.get("ended_at")?,
    })
}

// A REST client id that ended comes back to life when the same id calls again.
pub fn upsert(c: &Connection, client: &Client) -> Result<(), Error> {
    c.execute(
        &format!(
            "INSERT INTO client ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, label = excluded.label,
                 created_at = excluded.created_at, ended_at = NULL"
        ),
        params![
            Text(&client.id),
            Text(&client.kind),
            client.label,
            client.current_tab_id.as_ref().map(Text),
            client.created_at,
            client.ended_at,
        ],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn get(c: &Connection, client_id: &ClientId) -> Result<Client, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM client WHERE id = ?1"),
        params![Text(client_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("client {client_id}"))
}

pub fn end(c: &Connection, client_id: &ClientId, now: i64) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE client SET ended_at = ?2 WHERE id = ?1",
            params![Text(client_id), now],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("client {client_id}"))
}

pub fn active(c: &Connection) -> Result<Vec<Client>, Error> {
    query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM client WHERE ended_at IS NULL ORDER BY created_at, id"),
        [],
        decode,
    )
}

pub fn set_current_tab(
    c: &Connection,
    client_id: &ClientId,
    tab_id: Option<&TabId>,
) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE client SET current_tab_id = ?2 WHERE id = ?1",
            params![Text(client_id), tab_id.map(Text)],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("client {client_id}"))
}
