use brr_core::{
    BuildId, EndReason, Error, Instance, InstanceId, InstanceState, Page, ProfileId, SandboxMode,
    Traffic,
};
use rusqlite::{Connection, Row, ToSql, params, params_from_iter};

use crate::{
    Json, Text, col, expect_changed, first_or_not_found, internal, next_cursor, opt_col,
    opt_json_col, parse_cursor, query,
};

const TABLE: &str = "instance";
const COLUMNS: &str = "id, kind, profile_id, build_id, state, display, priority, admission, pid, log_path, cdp_url, data_dir, ephemeral, promote_to_name, restart_count, last_activity_at, queued_at, started_at, ended_at, end_reason, error, profile_name, settings, template_id, template_name, extensions, proxy_id, sandbox, sent_bytes, received_bytes, host_id";
// Must match InstanceState::is_final; the repos test checks every state against it.
pub(crate) const NON_FINAL: &str = "state NOT IN ('stopped', 'failed', 'detached', 'lost')";
// A final run is placed in time by when it ended, an open one by when it was queued.
const RUN_TIME: &str = "COALESCE(ended_at, queued_at)";

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub states: Vec<InstanceState>,
    pub profile_id: Option<ProfileId>,
    // An id or a name; the name still matches after the Template is deleted.
    pub template: Option<String>,
    pub end_reason: Option<EndReason>,
    pub since: Option<i64>,
    pub until: Option<i64>,
}

fn decode(row: &Row) -> rusqlite::Result<Instance> {
    Ok(Instance {
        id: col(row, "id")?,
        kind: col(row, "kind")?,
        profile_id: opt_col(row, "profile_id")?,
        build_id: opt_col(row, "build_id")?,
        state: col(row, "state")?,
        display: col(row, "display")?,
        priority: row.get("priority")?,
        admission: col(row, "admission")?,
        pid: row.get("pid")?,
        log_path: opt_col(row, "log_path")?,
        cdp_url: row.get("cdp_url")?,
        data_dir: opt_col(row, "data_dir")?,
        ephemeral: row.get("ephemeral")?,
        promote_to_name: row.get("promote_to_name")?,
        restart_count: row.get("restart_count")?,
        last_activity_at: row.get("last_activity_at")?,
        queued_at: row.get("queued_at")?,
        started_at: row.get("started_at")?,
        ended_at: row.get("ended_at")?,
        end_reason: opt_col(row, "end_reason")?,
        error: row.get("error")?,
        profile_name: row.get("profile_name")?,
        settings: opt_json_col(row, "settings")?,
        template_id: opt_col(row, "template_id")?,
        template_name: row.get("template_name")?,
        extensions: opt_json_col(row, "extensions")?,
        proxy_id: opt_col(row, "proxy_id")?,
        sandbox: row
            .get::<_, Option<String>>("sandbox")?
            .and_then(|mode| match mode.as_str() {
                "docker" => Some(SandboxMode::Docker),
                _ => None,
            }),
        traffic: traffic(row)?,
        host_id: opt_col(row, "host_id")?,
        build: None,
        tab_count: 0,
        lease_count: 0,
        rss_bytes: None,
        capabilities: Vec::new(),
    })
}

// NULL columns: the run had no forwarder, so nothing was measured.
fn traffic(row: &Row) -> rusqlite::Result<Option<Traffic>> {
    let sent: Option<i64> = row.get("sent_bytes")?;
    let received: Option<i64> = row.get("received_bytes")?;

    Ok(match (sent, received) {
        (Some(sent), Some(received)) => Some(Traffic {
            sent_bytes: u64::try_from(sent).unwrap_or_default(),
            received_bytes: u64::try_from(received).unwrap_or_default(),
        }),
        _ => None,
    })
}

// SQLite integers are signed 64-bit; a byte counter never gets near that.
fn to_sql_int(bytes: u64) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

pub fn set_traffic(
    c: &Connection,
    instance_id: &InstanceId,
    traffic: &Traffic,
) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE instance SET sent_bytes = ?2, received_bytes = ?3 WHERE id = ?1",
            params![
                Text(instance_id),
                to_sql_int(traffic.sent_bytes),
                to_sql_int(traffic.received_bytes)
            ],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("instance {instance_id}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficGroup {
    Profile,
    Proxy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrafficTotal {
    pub id: String,
    pub name: String,
    pub sent_bytes: u64,
    pub received_bytes: u64,
    pub runs: u64,
}

// Measured runs that ended at or after `from`, summed per Profile or Proxy. A Proxy has no name
// of its own, so it is shown by its endpoint.
pub fn traffic_totals(
    c: &Connection,
    group: TrafficGroup,
    from: i64,
) -> Result<Vec<TrafficTotal>, Error> {
    let (key, label, join) = match group {
        TrafficGroup::Profile => (
            "i.profile_id",
            "COALESCE(MAX(i.profile_name), i.profile_id)",
            "",
        ),
        TrafficGroup::Proxy => (
            "i.proxy_id",
            "COALESCE(MAX(p.host || ':' || p.port), i.proxy_id)",
            "LEFT JOIN proxy p ON p.id = i.proxy_id",
        ),
    };

    query(
        c,
        TABLE,
        &format!(
            "SELECT {key} AS id, {label} AS name, SUM(i.sent_bytes) AS sent, SUM(i.received_bytes) AS received, COUNT(*) AS runs
             FROM instance i {join}
             WHERE {key} IS NOT NULL AND i.sent_bytes IS NOT NULL AND i.ended_at >= ?1
             GROUP BY {key} ORDER BY sent + received DESC, id"
        ),
        params![from],
        |row| {
            Ok(TrafficTotal {
                id: row.get("id")?,
                name: row.get("name")?,
                sent_bytes: u64::try_from(row.get::<_, i64>("sent")?).unwrap_or_default(),
                received_bytes: u64::try_from(row.get::<_, i64>("received")?).unwrap_or_default(),
                runs: u64::try_from(row.get::<_, i64>("runs")?).unwrap_or_default(),
            })
        },
    )
}

// When each measured run ended and how many bytes it moved, for the runs that ended in [from, to).
pub fn traffic_ends(c: &Connection, from: i64, to: i64) -> Result<Vec<(i64, u64)>, Error> {
    query(
        c,
        TABLE,
        "SELECT ended_at, sent_bytes + received_bytes AS bytes FROM instance
         WHERE sent_bytes IS NOT NULL AND ended_at >= ?1 AND ended_at < ?2",
        params![from, to],
        |row| {
            Ok((
                row.get("ended_at")?,
                u64::try_from(row.get::<_, i64>("bytes")?).unwrap_or_default(),
            ))
        },
    )
}

pub fn insert(c: &Connection, i: &Instance) -> Result<(), Error> {
    c.execute(
        &format!("INSERT INTO instance ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31)"),
        params![
            Text(&i.id),
            Text(&i.kind),
            i.profile_id.as_ref().map(Text),
            i.build_id.as_ref().map(Text),
            Text(&i.state),
            Text(&i.display),
            i.priority,
            Text(&i.admission),
            i.pid,
            i.log_path.as_ref().map(Text),
            i.cdp_url,
            i.data_dir.as_ref().map(Text),
            i.ephemeral,
            i.promote_to_name,
            i.restart_count,
            i.last_activity_at,
            i.queued_at,
            i.started_at,
            i.ended_at,
            i.end_reason.as_ref().map(Text),
            i.error,
            i.profile_name,
            i.settings.as_ref().map(Json),
            i.template_id.as_ref().map(Text),
            i.template_name,
            i.extensions.as_ref().map(Json),
            i.proxy_id.as_ref().map(Text),
            i.sandbox.map(SandboxMode::as_str),
            i.traffic.as_ref().map(|t| to_sql_int(t.sent_bytes)),
            i.traffic.as_ref().map(|t| to_sql_int(t.received_bytes)),
            i.host_id.as_ref().map(Text),
        ],
    )
    .map_err(internal)?;

    Ok(())
}

pub fn update(c: &Connection, i: &Instance) -> Result<(), Error> {
    let changed = c
        .execute(
            "UPDATE instance SET kind = ?2, profile_id = ?3, build_id = ?4, state = ?5, display = ?6, priority = ?7,
                 admission = ?8, pid = ?9, log_path = ?10, cdp_url = ?11, data_dir = ?12, ephemeral = ?13,
                 promote_to_name = ?14, restart_count = ?15, last_activity_at = ?16, queued_at = ?17,
                 started_at = ?18, ended_at = ?19, end_reason = ?20, error = ?21, extensions = ?22, proxy_id = ?23,
                 sandbox = ?24, host_id = ?25
             WHERE id = ?1",
            params![
                Text(&i.id),
                Text(&i.kind),
                i.profile_id.as_ref().map(Text),
                i.build_id.as_ref().map(Text),
                Text(&i.state),
                Text(&i.display),
                i.priority,
                Text(&i.admission),
                i.pid,
                i.log_path.as_ref().map(Text),
                i.cdp_url,
                i.data_dir.as_ref().map(Text),
                i.ephemeral,
                i.promote_to_name,
                i.restart_count,
                i.last_activity_at,
                i.queued_at,
                i.started_at,
                i.ended_at,
                i.end_reason.as_ref().map(Text),
                i.error,
                i.extensions.as_ref().map(Json),
                i.proxy_id.as_ref().map(Text),
                i.sandbox.map(SandboxMode::as_str),
                i.host_id.as_ref().map(Text),
            ],
        )
        .map_err(internal)?;

    expect_changed(changed, format!("instance {}", i.id))
}

pub fn get(c: &Connection, instance_id: &InstanceId) -> Result<Instance, Error> {
    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM instance WHERE id = ?1"),
        params![Text(instance_id)],
        decode,
    )?;

    first_or_not_found(rows, format!("instance {instance_id}"))
}

pub fn list(
    c: &Connection,
    filter: &Filter,
    page: &Page,
) -> Result<(Vec<Instance>, Option<String>), Error> {
    let cursor = page
        .cursor
        .as_deref()
        .map(parse_cursor::<InstanceId>)
        .transpose()?;
    let mut clauses = vec!["id < ?".to_string()];
    // "~" sorts after every ULID character, so the first page has no lower bound.
    let mut args: Vec<Box<dyn ToSql + '_>> = vec![Box::new(
        cursor.map_or_else(|| "~".to_string(), |id| id.to_string()),
    )];

    if !filter.states.is_empty() {
        let marks = vec!["?"; filter.states.len()].join(", ");
        clauses.push(format!("state IN ({marks})"));
        args.extend(
            filter
                .states
                .iter()
                .map(|state| Box::new(Text(state)) as Box<dyn ToSql + '_>),
        );
    }

    if let Some(profile_id) = &filter.profile_id {
        clauses.push("profile_id = ?".to_string());
        args.push(Box::new(profile_id.to_string()));
    }

    if let Some(template) = &filter.template {
        clauses.push("(template_id = ? OR template_name = ?)".to_string());
        args.push(Box::new(template.as_str()));
        args.push(Box::new(template.as_str()));
    }

    if let Some(end_reason) = &filter.end_reason {
        clauses.push("end_reason = ?".to_string());
        args.push(Box::new(Text(end_reason)));
    }

    if let Some(since) = filter.since {
        clauses.push(format!("{RUN_TIME} >= ?"));
        args.push(Box::new(since));
    }

    if let Some(until) = filter.until {
        clauses.push(format!("{RUN_TIME} < ?"));
        args.push(Box::new(until));
    }

    args.push(Box::new(page.limit));

    let rows = query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM instance WHERE {} ORDER BY id DESC LIMIT ?",
            clauses.join(" AND ")
        ),
        params_from_iter(args.iter()),
        decode,
    )?;
    let next = next_cursor(&rows, page, |i| i.id.to_string());

    Ok((rows, next))
}

pub fn names_template(c: &Connection, template: &str) -> Result<bool, Error> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM instance WHERE template_id = ?1 OR template_name = ?1)",
        params![template],
        |row| row.get(0),
    )
    .map_err(internal)
}

pub fn active_for_profile(
    c: &Connection,
    profile_id: &ProfileId,
) -> Result<Option<Instance>, Error> {
    let rows = query(
        c,
        TABLE,
        &format!(
            "SELECT {COLUMNS} FROM instance WHERE profile_id = ?1 AND {NON_FINAL} ORDER BY id DESC LIMIT 1"
        ),
        params![Text(profile_id)],
        decode,
    )?;

    Ok(rows.into_iter().next())
}

pub fn non_final(c: &Connection) -> Result<Vec<Instance>, Error> {
    query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM instance WHERE {NON_FINAL} ORDER BY id"),
        [],
        decode,
    )
}

pub fn all_ids(c: &Connection) -> Result<Vec<InstanceId>, Error> {
    query(c, TABLE, "SELECT id FROM instance ORDER BY id", [], |row| {
        col(row, "id")
    })
}

pub fn using_build(c: &Connection, build_id: &BuildId) -> Result<Vec<InstanceId>, Error> {
    query(
        c,
        TABLE,
        &format!("SELECT id FROM instance WHERE build_id = ?1 AND {NON_FINAL} ORDER BY id"),
        params![Text(build_id)],
        |row| col(row, "id"),
    )
}

pub struct Span {
    // "chromium", "camoufox" or "attached" for a Live browser.
    pub engine: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

// Runs that were up at some point in [from, to); an open run has no end yet. A managed run whose
// Build was removed counts as Chromium, the engine of every Build but Camoufox.
pub fn spans(c: &Connection, from: i64, to: i64) -> Result<Vec<Span>, Error> {
    query(
        c,
        TABLE,
        &format!(
            "SELECT CASE WHEN i.kind = 'live' THEN 'attached' WHEN b.engine = 'camoufox' THEN 'camoufox' ELSE 'chromium' END AS engine,
                    i.started_at, i.ended_at
             FROM instance i LEFT JOIN build b ON b.id = i.build_id
             WHERE i.started_at IS NOT NULL AND i.started_at < ?2
               AND (i.ended_at > ?1 OR (i.ended_at IS NULL AND i.{NON_FINAL}))"
        ),
        params![from, to],
        |row| {
            Ok(Span {
                engine: row.get("engine")?,
                started_at: row.get("started_at")?,
                ended_at: row.get("ended_at")?,
            })
        },
    )
}
