use brr_core::{Error, Page, Proxy, ProxyId};
use rusqlite::{Connection, Row, params};

use crate::{
    Text, col, expect_changed, first_or_not_found, internal, name_conflict, next_cursor,
    parse_cursor, query,
};

const TABLE: &str = "proxy";
// password_enc is deliberately absent: only password_enc() reads it.
const COLUMNS: &str = "id, scheme, host, port, username, healthy, exit_ip, country, latency_ms, last_error, checked_at";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    pub healthy: bool,
    pub exit_ip: Option<String>,
    pub country: Option<String>,
    pub latency_ms: Option<u64>,
    pub last_error: Option<String>,
    pub checked_at: i64,
}

fn decode(row: &Row) -> rusqlite::Result<Proxy> {
    Ok(Proxy {
        id: col(row, "id")?,
        scheme: col(row, "scheme")?,
        host: row.get("host")?,
        port: row.get("port")?,
        username: row.get("username")?,
        password: None,
        healthy: row.get("healthy")?,
        exit_ip: row.get("exit_ip")?,
        country: row.get("country")?,
        latency_ms: row.get("latency_ms")?,
        last_error: row.get("last_error")?,
        checked_at: row.get("checked_at")?,
    })
}

pub fn insert(c: &Connection, p: &Proxy, password_enc: Option<&[u8]>) -> Result<(), Error> {
    c.execute(
        "INSERT INTO proxy (id, scheme, host, port, username, password_enc, healthy, exit_ip, country, latency_ms, last_error, checked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            Text(&p.id),
            Text(&p.scheme),
            p.host,
            p.port,
            p.username,
            password_enc,
            p.healthy,
            p.exit_ip,
            p.country,
            p.latency_ms,
            p.last_error,
            p.checked_at,
        ],
    )
    .map_err(name_conflict)?;

    Ok(())
}

pub fn update(c: &Connection, p: &Proxy) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE proxy SET scheme = ?2, host = ?3, port = ?4, username = ?5 WHERE id = ?1",
            params![Text(&p.id), Text(&p.scheme), p.host, p.port, p.username],
        )
        .map_err(name_conflict)?;

    expect_changed(changed, format!("proxy {}", p.id))
}

pub fn get(c: &Connection, proxy_id: &ProxyId) -> Result<Proxy, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM proxy WHERE id = ?1"),
        params![Text(proxy_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("proxy {proxy_id}"))
}

pub fn list(c: &Connection, page: &Page) -> Result<(Vec<Proxy>, Option<String>), Error> {
    let cursor = page
        .cursor
        .as_deref()
        .map(parse_cursor::<ProxyId>)
        .transpose()?;
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM proxy WHERE id > ?1 ORDER BY id LIMIT ?2"),
        params![
            cursor.map(|id| id.to_string()).unwrap_or_default(),
            page.limit
        ],
        decode,
    )?;
    let next = next_cursor(&rows, page, |p| p.id.to_string());

    Ok((rows, next))
}

pub fn all(c: &Connection) -> Result<Vec<Proxy>, Error> {
    query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM proxy ORDER BY id"),
        [],
        decode,
    )
}

pub fn delete(c: &Connection, proxy_id: &ProxyId) -> Result<(), Error> {
    let changed = c
        .execute("DELETE FROM proxy WHERE id = ?1", params![Text(proxy_id)])
        .map_err(internal)?;

    expect_changed(changed, format!("proxy {proxy_id}"))
}

pub fn password_enc(c: &Connection, proxy_id: &ProxyId) -> Result<Option<Vec<u8>>, Error> {
    c.query_row(
        "SELECT password_enc FROM proxy WHERE id = ?1",
        params![Text(proxy_id)],
        |row| row.get(0),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Error::NotFound {
            what: format!("proxy {proxy_id}"),
        },
        e => internal(e),
    })
}

pub fn set_password(
    c: &Connection,
    proxy_id: &ProxyId,
    value_enc: Option<&[u8]>,
) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE proxy SET password_enc = ?2 WHERE id = ?1",
            params![Text(proxy_id), value_enc],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("proxy {proxy_id}"))
}

pub fn set_health(c: &Connection, proxy_id: &ProxyId, h: &Health) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE proxy SET healthy = ?2, exit_ip = ?3, country = ?4, latency_ms = ?5, last_error = ?6, checked_at = ?7 WHERE id = ?1",
            params![
                Text(proxy_id),
                h.healthy,
                h.exit_ip,
                h.country,
                h.latency_ms,
                h.last_error,
                h.checked_at,
            ],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("proxy {proxy_id}"))
}

pub fn clear_health(c: &Connection, proxy_id: &ProxyId) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE proxy SET healthy = NULL, exit_ip = NULL, country = NULL, latency_ms = NULL, last_error = NULL, checked_at = NULL WHERE id = ?1",
            params![Text(proxy_id)],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("proxy {proxy_id}"))
}

// The same comparison as the proxy_endpoint unique index: no username and an empty one are one endpoint.
pub fn find_endpoint(c: &Connection, p: &Proxy) -> Result<Option<ProxyId>, Error> {
    let rows: Vec<ProxyId> = query(
        c,
        TABLE,
        "SELECT id FROM proxy WHERE scheme = ?1 AND host = ?2 AND port = ?3 AND IFNULL(username, '') = IFNULL(?4, '')",
        params![Text(&p.scheme), p.host, p.port, p.username],
        |row| col(row, "id"),
    )?;

    Ok(rows.into_iter().next())
}
