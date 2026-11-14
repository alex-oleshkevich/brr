use brr_core::{Error, Page, Pool, PoolId, ProfileId, ProxyId};
use rusqlite::{Connection, Row, params};

use crate::{
    Text, col, expect_changed, first_or_not_found, internal, json_col, name_conflict, next_cursor,
    query,
};

const TABLE: &str = "proxy_pool";
const COLUMNS: &str = "id, name, mode";
const MEMBERS: &str = "(SELECT json_group_array(proxy_id) FROM (SELECT proxy_id FROM pool_member WHERE pool_id = proxy_pool.id ORDER BY rowid)) AS proxy_ids";

fn decode(row: &Row) -> rusqlite::Result<Pool> {
    Ok(Pool {
        id: col(row, "id")?,
        name: row.get("name")?,
        mode: col(row, "mode")?,
        proxy_ids: json_col(row, "proxy_ids")?,
    })
}

fn replace_members(c: &Connection, p: &Pool) -> Result<(), Error> {
    c.execute(
        "DELETE FROM pool_member WHERE pool_id = ?1",
        params![Text(&p.id)],
    )
    .map_err(internal)?;

    let mut insert = c
        .prepare_cached("INSERT OR IGNORE INTO pool_member (pool_id, proxy_id) VALUES (?1, ?2)")
        .map_err(internal)?;

    for proxy_id in &p.proxy_ids {
        insert
            .execute(params![Text(&p.id), Text(proxy_id)])
            .map_err(internal)?;
    }

    c.execute(
        "DELETE FROM pool_sticky WHERE pool_id = ?1
           AND proxy_id NOT IN (SELECT proxy_id FROM pool_member WHERE pool_id = ?1)",
        params![Text(&p.id)],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn insert(c: &Connection, p: &Pool) -> Result<(), Error> {
    c.execute(
        "INSERT INTO proxy_pool (id, name, mode) VALUES (?1, ?2, ?3)",
        params![Text(&p.id), p.name, Text(&p.mode)],
    )
    .map_err(name_conflict)?;

    replace_members(c, p)
}

pub fn update(c: &Connection, p: &Pool) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE proxy_pool SET name = ?2, mode = ?3 WHERE id = ?1",
            params![Text(&p.id), p.name, Text(&p.mode)],
        )
        .map_err(name_conflict)?;
    expect_changed(changed, format!("pool {}", p.id))?;

    replace_members(c, p)
}

pub fn get(c: &Connection, pool_id: &PoolId) -> Result<Pool, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS}, {MEMBERS} FROM proxy_pool WHERE id = ?1"),
        params![Text(pool_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("pool {pool_id}"))
}

pub fn find(c: &Connection, id_or_name: &str) -> Result<Pool, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS}, {MEMBERS} FROM proxy_pool WHERE id = ?1 OR name = ?1"),
        params![id_or_name],
        decode,
    )?;

    first_or_not_found(rows, format!("pool {id_or_name}"))
}

pub fn list(c: &Connection, page: &Page) -> Result<(Vec<Pool>, Option<String>), Error> {
    let rows = query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS}, {MEMBERS} FROM proxy_pool WHERE name > ?1 ORDER BY name LIMIT ?2"
        ),
        params![page.cursor.as_deref().unwrap_or_default(), page.limit],
        decode,
    )?;
    let next = next_cursor(&rows, page, |p| p.name.clone());

    Ok((rows, next))
}

pub fn delete(c: &Connection, pool_id: &PoolId) -> Result<(), Error> {
    let changed = c
        .execute(
            "DELETE FROM proxy_pool WHERE id = ?1",
            params![Text(pool_id)],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("pool {pool_id}"))
}

pub fn names_with_proxy(c: &Connection, proxy_id: &ProxyId) -> Result<Vec<String>, Error> {
    query(
        c,
        TABLE,
        "SELECT name FROM proxy_pool WHERE id IN (SELECT pool_id FROM pool_member WHERE proxy_id = ?1) ORDER BY name",
        params![Text(proxy_id)],
        |row| row.get(0),
    )
}

pub fn sticky(
    c: &Connection,
    pool_id: &PoolId,
    profile_id: &ProfileId,
) -> Result<Option<ProxyId>, Error> {
    let rows = query(
        c,
        "pool_sticky",
        "SELECT proxy_id FROM pool_sticky WHERE pool_id = ?1 AND profile_id = ?2",
        params![Text(pool_id), Text(profile_id)],
        |row| col(row, "proxy_id"),
    )?;

    Ok(rows.into_iter().next())
}

pub fn set_sticky(
    c: &Connection,
    pool_id: &PoolId,
    profile_id: &ProfileId,
    proxy_id: &ProxyId,
) -> Result<(), Error> {
    c.execute(
        "INSERT INTO pool_sticky (pool_id, profile_id, proxy_id) VALUES (?1, ?2, ?3)
         ON CONFLICT(pool_id, profile_id) DO UPDATE SET proxy_id = excluded.proxy_id",
        params![Text(pool_id), Text(profile_id), Text(proxy_id)],
    )
    .map_err(internal)?;

    Ok(())
}
