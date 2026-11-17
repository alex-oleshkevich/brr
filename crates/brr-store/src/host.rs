use brr_core::{Error, Host, HostId, HostStatus};
use rusqlite::{Connection, Row, params};

use crate::{
    Text, col, expect_changed, first_or_not_found, instance::NON_FINAL, internal, name_conflict,
    query,
};

const TABLE: &str = "host";

fn select(filter: &str) -> String {
    format!(
        "SELECT id, name, docker_url, max_instances, enabled, created_at,
           (SELECT COUNT(*) FROM instance WHERE instance.host_id = host.id AND {NON_FINAL}) AS running
         FROM host {filter}"
    )
}

fn decode(row: &Row) -> rusqlite::Result<Host> {
    Ok(Host {
        id: col(row, "id")?,
        name: row.get("name")?,
        docker_url: row.get("docker_url")?,
        max_instances: row.get("max_instances")?,
        enabled: row.get("enabled")?,
        created_at: row.get("created_at")?,
        status: HostStatus {
            running: row.get("running")?,
            ..HostStatus::default()
        },
    })
}

pub fn insert(c: &Connection, h: &Host) -> Result<(), Error> {
    c.execute(
        "INSERT INTO host (id, name, docker_url, max_instances, enabled, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            Text(&h.id),
            h.name,
            h.docker_url,
            h.max_instances,
            h.enabled,
            h.created_at,
        ],
    )
    .map_err(name_conflict)?;

    Ok(())
}

pub fn update(c: &Connection, h: &Host) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE host SET name = ?2, docker_url = ?3, max_instances = ?4, enabled = ?5 WHERE id = ?1",
            params![Text(&h.id), h.name, h.docker_url, h.max_instances, h.enabled],
        )
        .map_err(name_conflict)?;

    expect_changed(changed, format!("host {}", h.id))
}

pub fn get(c: &Connection, host_id: &HostId) -> Result<Host, Error> {
    let rows = query(
        c,
        TABLE,
        &select("WHERE id = ?1"),
        params![Text(host_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("host {host_id}"))
}

pub fn find(c: &Connection, id_or_name: &str) -> Result<Host, Error> {
    let rows = query(
        c,
        TABLE,
        &select("WHERE id = ?1 OR name = ?1"),
        params![id_or_name],
        decode,
    )?;

    first_or_not_found(rows, format!("host {id_or_name}"))
}

pub fn all(c: &Connection) -> Result<Vec<Host>, Error> {
    query(c, TABLE, &select("ORDER BY name"), [], decode)
}

pub fn delete(c: &Connection, host_id: &HostId) -> Result<(), Error> {
    let changed = c
        .execute("DELETE FROM host WHERE id = ?1", params![Text(host_id)])
        .map_err(internal)?;

    expect_changed(changed, format!("host {host_id}"))
}
