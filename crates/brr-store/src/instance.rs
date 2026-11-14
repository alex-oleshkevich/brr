use brr_core::{BuildId, Error, Instance, InstanceId, InstanceState, Page, ProfileId};
use rusqlite::{Connection, Row, ToSql, params, params_from_iter};

use crate::{Text, col, expect_changed, first_or_not_found, internal, next_cursor, opt_col, query};

const TABLE: &str = "instance";
const COLUMNS: &str = "id, kind, profile_id, build_id, state, display, priority, admission, pid, log_path, cdp_url, data_dir, ephemeral, promote_to_name, restart_count, last_activity_at, queued_at, started_at, ended_at, end_reason, error, profile_name";
// Must match InstanceState::is_final; the repos test checks every state against it.
pub(crate) const NON_FINAL: &str = "state NOT IN ('stopped', 'failed', 'detached', 'lost')";

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub states: Vec<InstanceState>,
    pub profile_id: Option<ProfileId>,
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
        build: None,
        tab_count: 0,
        lease_count: 0,
        rss_bytes: None,
        capabilities: Vec::new(),
    })
}

pub fn insert(c: &Connection, i: &Instance) -> Result<(), Error> {
    c.execute(
        &format!("INSERT INTO instance ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)"),
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
                 started_at = ?18, ended_at = ?19, end_reason = ?20, error = ?21
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
    let mut clauses = vec!["id < ?".to_string()];
    // "~" sorts after every ULID character, so the first page has no lower bound.
    let mut args: Vec<Box<dyn ToSql + '_>> = vec![Box::new(
        page.cursor.clone().unwrap_or_else(|| "~".to_string()),
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
