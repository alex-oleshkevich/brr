use brr_core::Error;
use rusqlite::{Connection, params};

use crate::{internal, query};

const TABLE: &str = "secret";

pub fn put(c: &Connection, name: &str, value_enc: &[u8], now: i64) -> Result<(), Error> {
    c.execute(
        "INSERT INTO secret (name, value_enc, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(name) DO UPDATE SET value_enc = excluded.value_enc, updated_at = excluded.updated_at",
        params![name, value_enc, now],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn get(c: &Connection, name: &str) -> Result<Option<Vec<u8>>, Error> {
    let rows = query(
        c,
        TABLE,
        "SELECT value_enc FROM secret WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )?;

    Ok(rows.into_iter().next())
}

pub fn delete(c: &Connection, name: &str) -> Result<(), Error> {
    c.execute("DELETE FROM secret WHERE name = ?1", params![name])
        .map_err(internal)?;

    Ok(())
}

pub fn names(c: &Connection) -> Result<Vec<String>, Error> {
    query(
        c,
        TABLE,
        "SELECT name FROM secret ORDER BY name",
        [],
        |row| row.get(0),
    )
}
