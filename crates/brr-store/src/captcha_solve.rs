use brr_core::{CaptchaSolve, Error, Page};
use rusqlite::{Connection, Row, params};

use crate::{Text, col, internal, next_cursor, opt_col, parse_cursor, parse_cursor_pair, query};

const TABLE: &str = "captcha_solve";
const COLUMNS: &str =
    "id, ts, profile_id, tab_id, site, type, provider, cost_usd_micros, duration_ms, result, error";

fn decode(row: &Row) -> rusqlite::Result<CaptchaSolve> {
    Ok(CaptchaSolve {
        id: col(row, "id")?,
        ts: row.get("ts")?,
        profile_id: opt_col(row, "profile_id")?,
        tab_id: opt_col(row, "tab_id")?,
        site: row.get("site")?,
        captcha_type: col(row, "type")?,
        provider: row.get("provider")?,
        cost_usd_micros: row.get("cost_usd_micros")?,
        duration_ms: row.get("duration_ms")?,
        result: col(row, "result")?,
        error: row.get("error")?,
    })
}

pub fn insert(c: &Connection, solve: &CaptchaSolve) -> Result<(), Error> {
    c.execute(
        &format!("INSERT INTO captcha_solve ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"),
        params![
            Text(&solve.id),
            solve.ts,
            solve.profile_id.as_ref().map(Text),
            solve.tab_id.as_ref().map(Text),
            solve.site,
            Text(&solve.captcha_type),
            solve.provider,
            solve.cost_usd_micros,
            solve.duration_ms,
            Text(&solve.result),
            solve.error,
        ],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn list(c: &Connection, page: &Page) -> Result<(Vec<CaptchaSolve>, Option<String>), Error> {
    let after = match page.cursor.as_deref().map(parse_cursor_pair).transpose()? {
        Some((ts, id)) => Some((parse_cursor::<i64>(&ts)?, id)),
        None => None,
    };
    let rows = query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM captcha_solve WHERE ?1 IS NULL OR (ts, id) < (?1, ?2) ORDER BY ts DESC, id DESC LIMIT ?3"
        ),
        params![
            after.as_ref().map(|(ts, _)| ts),
            after.as_ref().map(|(_, id)| id),
            page.limit
        ],
        decode,
    )?;
    let next = next_cursor(&rows, page, |s| format!("{}\u{1f}{}", s.ts, s.id));

    Ok((rows, next))
}

pub fn spent_since(c: &Connection, since_ms: i64) -> Result<(u64, u32), Error> {
    c.query_row(
        "SELECT COALESCE(SUM(cost_usd_micros), 0), COUNT(*) FROM captcha_solve WHERE ts >= ?1",
        params![since_ms],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(internal)
}
