use std::path::PathBuf;

use brr_core::{Error, ExtensionId};
use rusqlite::{Connection, params};

use crate::{col, instance::NON_FINAL, internal, query};

const FINISHED_ROWS_MAX_DAYS: i64 = 30;
const MS_PER_DAY: i64 = 86_400_000;
pub const FINISHED_ROWS_MAX_AGE_MS: i64 = FINISHED_ROWS_MAX_DAYS * MS_PER_DAY;

fn count(c: &Connection, sql: &str, ts: i64) -> Result<u32, Error> {
    c.query_row(sql, params![ts], |row| row.get(0))
        .map_err(internal)
}

fn delete(c: &Connection, sql: &str, ts: i64) -> Result<u32, Error> {
    let deleted = c.execute(sql, params![ts]).map_err(internal)?;

    u32::try_from(deleted).map_err(internal)
}

pub fn events_before(c: &Connection, ts: i64) -> Result<u32, Error> {
    count(c, "SELECT count(*) FROM event WHERE ts < ?1", ts)
}

// By ts, never seq: the two orders agree only within one daemon run.
pub fn delete_events_before(c: &Connection, ts: i64) -> Result<u32, Error> {
    delete(c, "DELETE FROM event WHERE ts < ?1", ts)
}

pub fn captcha_solves_before(c: &Connection, ts: i64) -> Result<u32, Error> {
    count(c, "SELECT count(*) FROM captcha_solve WHERE ts < ?1", ts)
}

pub fn delete_captcha_solves_before(c: &Connection, ts: i64) -> Result<u32, Error> {
    delete(c, "DELETE FROM captcha_solve WHERE ts < ?1", ts)
}

pub fn extension_dirs(c: &Connection) -> Result<Vec<(ExtensionId, String, PathBuf)>, Error> {
    query(
        c,
        "extension_version",
        "SELECT e.id AS id, e.name AS name, v.path AS path FROM extension e JOIN extension_version v ON v.extension_id = e.id ORDER BY e.id, v.version",
        [],
        |row| Ok((col(row, "id")?, row.get("name")?, col(row, "path")?)),
    )
}

// Final Instances (their Tabs cascade), released Leases and ended Clients are history only;
// without this sweep they grow without bound. Rows that asked for a promotion stay: the
// daemon's startup reconcile reads them to finish the promotion or set the data aside, and it
// runs after this sweep.
pub fn sweep_finished_rows(c: &Connection, cutoff: i64) -> Result<u32, Error> {
    let instances = delete(
        c,
        &format!(
            "DELETE FROM instance WHERE ended_at < ?1 AND promote_to_name IS NULL AND NOT ({NON_FINAL})"
        ),
        cutoff,
    )?;
    let leases = delete(c, "DELETE FROM lease WHERE released_at < ?1", cutoff)?;
    let clients = delete(c, "DELETE FROM client WHERE ended_at < ?1", cutoff)?;

    Ok(instances + leases + clients)
}
