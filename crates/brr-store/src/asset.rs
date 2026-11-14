use brr_core::{Asset, AssetId, AssetKind, Error};
use rusqlite::{Connection, Row, params};

use crate::{Text, col, expect_changed, internal, query};

const TABLE: &str = "asset";
const COLUMNS: &str = "id, kind, version, path, updated_at";

fn decode(row: &Row) -> rusqlite::Result<Asset> {
    Ok(Asset {
        id: col(row, "id")?,
        kind: col(row, "kind")?,
        version: row.get("version")?,
        path: col(row, "path")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn put(c: &Connection, a: &Asset) -> Result<(), Error> {
    c.execute(
        &format!("INSERT OR REPLACE INTO asset ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5)"),
        params![
            Text(&a.id),
            Text(&a.kind),
            a.version,
            Text(&a.path),
            a.updated_at
        ],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn by_kind(c: &Connection, kind: AssetKind) -> Result<Vec<Asset>, Error> {
    query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM asset WHERE kind = ?1 ORDER BY id"),
        params![Text(&kind)],
        decode,
    )
}

pub fn delete(c: &Connection, asset_id: &AssetId) -> Result<(), Error> {
    let changed = c
        .execute("DELETE FROM asset WHERE id = ?1", params![Text(asset_id)])
        .map_err(internal)?;

    expect_changed(changed, format!("asset {asset_id}"))
}
