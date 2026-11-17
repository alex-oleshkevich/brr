pub mod artifact;
pub mod asset;
pub mod backup;
pub mod build;
pub mod captcha_solve;
pub mod client;
pub mod credential;
pub mod event;
pub mod extension;
pub mod host;
pub mod instance;
pub mod lease;
pub mod login_check;
pub mod pool;
pub mod profile;
pub mod profile_extension;
pub mod proxy;
pub mod retention;
pub mod secret;
pub mod tab;
pub mod template;
pub mod template_extension;

use std::{
    fmt::Display,
    path::Path,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use brr_core::{ConflictReason, Error, Page, now_ms};
use rusqlite::{
    Params, Row, ToSql,
    ffi::SQLITE_CONSTRAINT_UNIQUE,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
};
use rusqlite_migration::{M, Migrations};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

pub use rusqlite::{Connection, Transaction};

const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(db_file: &Path) -> Result<Store, Error> {
        if let Some(parent) = db_file.parent() {
            std::fs::create_dir_all(parent).map_err(internal)?;
        }

        let store = Store::init(Connection::open(db_file).map_err(internal)?)?;
        tracing::info!(db_file = %db_file.display(), "store opened");

        Ok(store)
    }

    pub fn open_in_memory() -> Result<Store, Error> {
        Store::init(Connection::open_in_memory().map_err(internal)?)
    }

    pub async fn run<T, F>(&self, f: F) -> Result<T, Error>
    where
        F: FnOnce(&mut Connection) -> Result<T, Error> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);

        tokio::task::spawn_blocking(move || {
            // A panic in an earlier closure leaves the connection usable: rusqlite rolls back
            // an open transaction on drop.
            let mut conn = conn.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut conn)
        })
        .await
        .map_err(internal)?
    }

    fn init(mut conn: Connection) -> Result<Store, Error> {
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(internal)?;
        // Off while migrating: SQLite ignores the pragma inside a transaction, and a table
        // rebuild must not cascade through the tables that reference it.
        conn.pragma_update(None, "foreign_keys", "OFF")
            .map_err(internal)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(internal)?;
        conn.busy_timeout(BUSY_TIMEOUT).map_err(internal)?;

        Migrations::new(vec![
            M::up(include_str!("migrations/0001_init.sql")),
            M::up(include_str!("migrations/0002_m3.sql")),
            M::up(include_str!("migrations/0003_retention_indexes.sql")),
            M::up(include_str!("migrations/0004_ux.sql")),
            M::up(include_str!("migrations/0005_history.sql")),
            M::up(include_str!("migrations/0006_sandbox.sql")),
            M::up(include_str!("migrations/0007_bitwarden.sql")).foreign_key_check(),
            M::up(include_str!("migrations/0008_traffic.sql")),
            M::up(include_str!("migrations/0009_hosts.sql")).foreign_key_check(),
        ])
        .to_latest(&mut conn)
        .map_err(internal)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(internal)?;

        let swept =
            retention::sweep_finished_rows(&conn, now_ms() - retention::FINISHED_ROWS_MAX_AGE_MS)?;
        if swept > 0 {
            tracing::info!(swept, "finished rows deleted");
        }

        Ok(Store {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
}

fn internal(e: impl Display) -> Error {
    Error::Internal {
        message: e.to_string(),
    }
}

fn name_conflict(e: rusqlite::Error) -> Error {
    match e.sqlite_error() {
        Some(sqlite) if sqlite.extended_code == SQLITE_CONSTRAINT_UNIQUE => Error::Conflict {
            reason: ConflictReason::NameTaken,
            detail: None,
        },
        _ => internal(e),
    }
}

fn expect_changed(changed: usize, what: impl Display) -> Result<(), Error> {
    if changed == 0 {
        return Err(Error::NotFound {
            what: what.to_string(),
        });
    }

    Ok(())
}

fn first_or_not_found<T>(rows: Vec<T>, what: impl Display) -> Result<T, Error> {
    rows.into_iter().next().ok_or_else(|| Error::NotFound {
        what: what.to_string(),
    })
}

fn next_cursor<T>(rows: &[T], page: &Page, key: impl Fn(&T) -> String) -> Option<String> {
    if rows.len() == page.limit as usize {
        return rows.last().map(key);
    }

    None
}

fn parse_cursor<T: std::str::FromStr>(cursor: &str) -> Result<T, Error> {
    cursor.parse().map_err(|_| bad_cursor())
}

fn bad_cursor() -> Error {
    Error::InvalidArgument {
        field: "cursor".to_string(),
        message: "not a cursor from a previous page; pass next_cursor from the previous page, or omit cursor for the first page".to_string(),
    }
}

fn parse_cursor_pair(cursor: &str) -> Result<(String, String), Error> {
    cursor
        .rsplit_once('\u{1f}')
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .ok_or_else(bad_cursor)
}

fn query<T>(
    c: &Connection,
    table: &str,
    sql: &str,
    params: impl Params,
    decode: fn(&Row) -> rusqlite::Result<T>,
) -> Result<Vec<T>, Error> {
    let mut stmt = c.prepare_cached(sql).map_err(internal)?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(decode(row).map_err(|e| corrupt(table, row, e)))
        })
        .map_err(internal)?;

    rows.map(|row| row.map_err(internal)?).collect()
}

fn corrupt(table: &str, row: &Row, e: rusqlite::Error) -> Error {
    let id = match row.get_ref(0) {
        Ok(ValueRef::Text(id)) => String::from_utf8_lossy(id).into_owned(),
        Ok(ValueRef::Integer(id)) => id.to_string(),
        _ => "?".to_string(),
    };

    internal(format!("corrupt {table} row {id}: {e}"))
}

// Enum names, ULIDs, channels and paths are stored as the text their serde form produces.
struct Text<T>(T);

impl<T: Serialize> ToSql for Text<&T> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        match serde_json::to_value(self.0) {
            Ok(Value::String(text)) => Ok(text.into()),
            Ok(other) => Err(rusqlite::Error::ToSqlConversionFailure(
                format!("expected a string, got {other}").into(),
            )),
            Err(e) => Err(rusqlite::Error::ToSqlConversionFailure(e.into())),
        }
    }
}

impl<T: DeserializeOwned> FromSql for Text<T> {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Text<T>> {
        let text = value.as_str()?.to_string();

        serde_json::from_value(Value::String(text))
            .map(Text)
            .map_err(|e| FromSqlError::Other(e.into()))
    }
}

struct Json<T>(T);

impl<T: Serialize> ToSql for Json<&T> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        serde_json::to_string(self.0)
            .map(Into::into)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))
    }
}

impl<T: DeserializeOwned> FromSql for Json<T> {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Json<T>> {
        serde_json::from_str(value.as_str()?)
            .map(Json)
            .map_err(|e| FromSqlError::Other(e.into()))
    }
}

fn col<T: DeserializeOwned>(row: &Row, name: &str) -> rusqlite::Result<T> {
    row.get::<_, Text<T>>(name).map(|text| text.0)
}

fn json_col<T: DeserializeOwned>(row: &Row, name: &str) -> rusqlite::Result<T> {
    row.get::<_, Json<T>>(name).map(|json| json.0)
}

fn opt_col<T: DeserializeOwned>(row: &Row, name: &str) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<Text<T>>>(name)
        .map(|text| text.map(|text| text.0))
}

fn opt_json_col<T: DeserializeOwned>(row: &Row, name: &str) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<Json<T>>>(name)
        .map(|json| json.map(|json| json.0))
}
