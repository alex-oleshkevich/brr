use brr_core::{Error, Event, InstanceId, Order, Page, Subject};
use rusqlite::{Connection, Row, ToSql, params, params_from_iter};
use serde_json::Value;

use crate::{Json, Text, col, internal, json_col, next_cursor, opt_col, parse_cursor, query};

const TABLE: &str = "event";
const COLUMNS: &str = "seq, ts, kind, subject_type, subject_id, actor_client_id, data";

// Both branches are indexed (event_subject, event_instance); an OR would scan the log.
const INSTANCE_FILTER: &str = "seq IN (SELECT seq FROM event WHERE subject_type = 'instance' AND subject_id = ? UNION SELECT seq FROM event WHERE json_extract(data, '$.instance_id') = ?)";

#[derive(Debug, Clone)]
pub struct Query {
    pub since: Option<i64>,
    pub kinds: Vec<String>,
    pub subject: Option<Subject>,
    pub instance_id: Option<InstanceId>,
    pub from_ts: Option<i64>,
    pub to_ts: Option<i64>,
    pub order: Order,
    pub page: Page,
}

fn decode(row: &Row) -> rusqlite::Result<Event> {
    Ok(Event {
        seq: row.get("seq")?,
        ts: row.get("ts")?,
        kind: row.get("kind")?,
        subject: Subject {
            kind: col(row, "subject_type")?,
            id: row.get("subject_id")?,
        },
        actor_client_id: opt_col(row, "actor_client_id")?,
        data: json_col::<Value>(row, "data")?,
    })
}

// The caller assigns seq, so the log order is the order the bus published.
pub fn insert(c: &Connection, e: &Event) -> Result<(), Error> {
    c.execute(
        &format!("INSERT INTO event ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
        params![
            e.seq,
            e.ts,
            e.kind,
            Text(&e.subject.kind),
            e.subject.id,
            e.actor_client_id.as_ref().map(Text),
            Json(&e.data)
        ],
    )
    .map_err(internal)?;

    Ok(())
}

// sqlite_sequence outlives deleted rows and holds reservations for transient events.
pub fn max_seq(c: &Connection) -> Result<i64, Error> {
    let seqs = query(
        c,
        TABLE,
        "SELECT seq FROM sqlite_sequence WHERE name = 'event'",
        [],
        |row| row.get(0),
    )?;

    Ok(seqs.into_iter().next().unwrap_or(0))
}

pub fn reserve_seq(c: &Connection, seq: i64) -> Result<(), Error> {
    let updated = c
        .execute(
            "UPDATE sqlite_sequence SET seq = MAX(seq, ?1) WHERE name = 'event'",
            params![seq],
        )
        .map_err(internal)?;

    if updated == 0 {
        c.execute(
            "INSERT INTO sqlite_sequence (name, seq) VALUES ('event', ?1)",
            params![seq],
        )
        .map_err(internal)?;
    }

    Ok(())
}

pub fn list(c: &Connection, q: &Query) -> Result<(Vec<Event>, Option<String>), Error> {
    let order = if q.since.is_some() {
        Order::Asc
    } else {
        q.order
    };
    let mut clauses: Vec<&str> = Vec::new();
    let mut args: Vec<Box<dyn ToSql + '_>> = Vec::new();

    if let Some(since) = q.since {
        clauses.push("seq > ?");
        args.push(Box::new(since));
    }

    if let Some(cursor) = &q.page.cursor {
        clauses.push(match order {
            Order::Asc => "seq > ?",
            Order::Desc => "seq < ?",
        });
        args.push(Box::new(parse_cursor::<i64>(cursor)?));
    }

    let kinds_clause;

    if !q.kinds.is_empty() {
        kinds_clause = format!("kind IN ({})", vec!["?"; q.kinds.len()].join(", "));
        clauses.push(&kinds_clause);
        args.extend(
            q.kinds
                .iter()
                .map(|kind| Box::new(kind) as Box<dyn ToSql + '_>),
        );
    }

    if let Some(subject) = &q.subject {
        clauses.push("subject_type = ? AND subject_id = ?");
        args.push(Box::new(Text(&subject.kind)));
        args.push(Box::new(&subject.id));
    }

    if let Some(instance_id) = &q.instance_id {
        clauses.push(INSTANCE_FILTER);
        args.push(Box::new(instance_id.to_string()));
        args.push(Box::new(instance_id.to_string()));
    }

    if let Some(from_ts) = q.from_ts {
        clauses.push("ts >= ?");
        args.push(Box::new(from_ts));
    }

    if let Some(to_ts) = q.to_ts {
        clauses.push("ts <= ?");
        args.push(Box::new(to_ts));
    }

    args.push(Box::new(q.page.limit));

    let filter = if clauses.is_empty() {
        "1".to_string()
    } else {
        clauses.join(" AND ")
    };
    let direction = match order {
        Order::Asc => "ASC",
        Order::Desc => "DESC",
    };

    let rows = query(
        c,
        TABLE,
        &format!("SELECT {COLUMNS} FROM event WHERE {filter} ORDER BY seq {direction} LIMIT ?"),
        params_from_iter(args.iter()),
        decode,
    )?;
    let next = next_cursor(&rows, &q.page, |e| e.seq.to_string());

    Ok((rows, next))
}
