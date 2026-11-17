use brr_core::{ClientId, Error, InstanceId, Lease, LeaseId, ReleaseReason};
use rusqlite::{Connection, Row, params};

use crate::{Text, col, internal, opt_col, query};

const TABLE: &str = "lease";
const COLUMNS: &str = "id, client_id, client_kind, client_label, instance_id, tab_id, exclusive, acquired_at, released_at, release_reason";

fn decode(row: &Row) -> rusqlite::Result<Lease> {
    Ok(Lease {
        id: col(row, "id")?,
        client_id: col(row, "client_id")?,
        client_kind: col(row, "client_kind")?,
        client_label: row.get("client_label")?,
        instance_id: col(row, "instance_id")?,
        tab_id: opt_col(row, "tab_id")?,
        exclusive: row.get("exclusive")?,
        acquired_at: row.get("acquired_at")?,
        released_at: row.get("released_at")?,
        release_reason: opt_col(row, "release_reason")?,
    })
}

// The partial unique index lease_active_tab rejects a second active lease on the same tab.
pub fn insert(c: &Connection, l: &Lease) -> Result<(), Error> {
    c.execute(
        &format!("INSERT INTO lease ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"),
        params![
            Text(&l.id),
            Text(&l.client_id),
            Text(&l.client_kind),
            l.client_label,
            Text(&l.instance_id),
            l.tab_id.as_ref().map(Text),
            l.exclusive,
            l.acquired_at,
            l.released_at,
            l.release_reason.as_ref().map(Text),
        ],
    )
    .map_err(internal)?;

    Ok(())
}

// Releasing twice is not an error: instance stop and client end can race to the same lease.
pub fn release(
    c: &Connection,
    lease_id: &LeaseId,
    reason: ReleaseReason,
    now: i64,
) -> Result<(), Error> {
    c.execute(
        "UPDATE lease SET released_at = ?2, release_reason = ?3 WHERE id = ?1 AND released_at IS NULL",
        params![Text(lease_id), now, Text(&reason)],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn active(c: &Connection) -> Result<Vec<Lease>, Error> {
    query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM lease WHERE released_at IS NULL ORDER BY acquired_at, id"),
        [],
        decode,
    )
}

// Active and released Leases, keyset-paginated on (acquired_at, id) after `after`.
pub fn history(
    c: &Connection,
    instance_id: Option<&InstanceId>,
    client_id: Option<&ClientId>,
    after: Option<(i64, LeaseId)>,
    limit: u32,
) -> Result<Vec<Lease>, Error> {
    let (after_at, after_id) =
        after.map_or((i64::MIN, String::new()), |(at, id)| (at, id.to_string()));

    query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM lease
             WHERE (?1 IS NULL OR instance_id = ?1) AND (?2 IS NULL OR client_id = ?2)
               AND (acquired_at, id) > (?3, ?4)
             ORDER BY acquired_at, id LIMIT ?5"
        ),
        params![
            instance_id.map(Text),
            client_id.map(Text),
            after_at,
            after_id,
            limit
        ],
        decode,
    )
}

pub fn release_all(c: &Connection, reason: ReleaseReason, now: i64) -> Result<usize, Error> {
    c.execute(
        "UPDATE lease SET released_at = ?1, release_reason = ?2 WHERE released_at IS NULL",
        params![now, Text(&reason)],
    )
    .map_err(internal)
}
