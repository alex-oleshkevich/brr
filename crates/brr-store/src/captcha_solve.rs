use brr_core::{CaptchaSolve, Error, InstanceId, Page};
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

// A solve is placed in its Instance through its Tab; Tab rows outlive the run until retention.
pub fn list(
    c: &Connection,
    instance_id: Option<&InstanceId>,
    page: &Page,
) -> Result<(Vec<CaptchaSolve>, Option<String>), Error> {
    let after = match page.cursor.as_deref().map(parse_cursor_pair).transpose()? {
        Some((ts, id)) => Some((parse_cursor::<i64>(&ts)?, id)),
        None => None,
    };
    let rows = query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM captcha_solve
             WHERE (?1 IS NULL OR (ts, id) < (?1, ?2))
               AND (?3 IS NULL OR tab_id IN (SELECT id FROM tab WHERE instance_id = ?3))
             ORDER BY ts DESC, id DESC LIMIT ?4"
        ),
        params![
            after.as_ref().map(|(ts, _)| ts),
            after.as_ref().map(|(_, id)| id),
            instance_id.map(Text),
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

pub struct DayTotals {
    pub day: String,
    pub spend_usd_micros: u64,
    pub solves: u32,
    pub failures: u32,
}

// SQLite's 'localtime' reads the same TZ as the daemon's clock, so the day keys line up.
pub fn daily_since(c: &Connection, since_ms: i64) -> Result<Vec<DayTotals>, Error> {
    query(
        c,
        TABLE,
        "SELECT date(ts / 1000, 'unixepoch', 'localtime') AS day, COALESCE(SUM(cost_usd_micros), 0) AS spend,
                SUM(result = 'solved') AS solves, SUM(result = 'failed') AS failures
         FROM captcha_solve WHERE ts >= ?1 GROUP BY day ORDER BY day",
        params![since_ms],
        |row| {
            Ok(DayTotals {
                day: row.get("day")?,
                spend_usd_micros: row.get("spend")?,
                solves: row.get("solves")?,
                failures: row.get("failures")?,
            })
        },
    )
}
