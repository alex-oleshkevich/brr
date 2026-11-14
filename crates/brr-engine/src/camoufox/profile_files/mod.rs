use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use brr_core::{Bookmark, Cookie, Error, SameSite, bookmark};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use super::internal;

#[cfg(test)]
mod tests;

const COOKIES_FILE: &str = "cookies.sqlite";
const PLACES_FILE: &str = "places.sqlite";
const PROFILE_FIELD: &str = "profile";
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);
const MICROS: i64 = 1_000_000;
// cookies.sqlite stores expiry in milliseconds.
const MILLIS: i64 = 1000;
// Decision (spec P4, profile cookies): Firefox keeps no session cookies in cookies.sqlite, and a
// login only survives into a stopped Profile if it is stored, so a session cookie written here
// becomes a persistent one that expires in 400 days, Chrome's own cap. The conversion is lossy:
// reading it back gives that date, not -1.
const SESSION_COOKIE_LIFETIME_S: i64 = 400 * 24 * 60 * 60;
const SAME_SITE_NONE: i64 = 0;
const SAME_SITE_LAX: i64 = 1;
const SAME_SITE_STRICT: i64 = 2;
// nsICookie::SAMESITE_UNSET: a cookie with no SameSite attribute. Juggler reports it as None, and
// laxByDefault is off in Camoufox, so Firefox sends it cross-site.
const SAME_SITE_UNSET: i64 = 256;
// Places hashes at most this many bytes of a URL, and looks for the scheme's colon in the first
// SCHEME_SCAN_BYTES of them.
const MAX_HASHED_BYTES: usize = 1500;
const SCHEME_SCAN_BYTES: usize = 50;
// The three roots a user sees, with the guids Firefox gives them and the names it shows.
const ROOTS: [(&str, &str); 3] = [
    ("menu________", "Bookmarks Menu"),
    ("toolbar_____", "Bookmarks Toolbar"),
    ("unfiled_____", "Other Bookmarks"),
];
const IMPORT_ROOT: &str = ROOTS[0].0;
const IMPORT_FOLDER: &str = "Imported";
const TYPE_BOOKMARK: i64 = 1;
const TYPE_FOLDER: i64 = 2;
const MAX_DEPTH: usize = 64;
const GUID_LEN: usize = 12;
const GUID_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
// Smart bookmarks are queries, not pages.
const QUERY_SCHEME: &str = "place:";
const GOLDEN_RATIO: u32 = 0x9E37_79B9;
const PREFIX_HASH_MASK: u32 = 0xFFFF;

fn now_micros() -> i64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    i64::try_from(since_epoch.as_micros()).unwrap_or(i64::MAX)
}

// Reads never take the write lock; the Profile is stopped, but a stray reader must not corrupt.
fn open(profile_dir: &Path, file: &str, flags: OpenFlags) -> Result<Connection, Error> {
    let path = profile_dir.join(file);
    if !path.exists() {
        return Err(Error::InvalidArgument {
            field: PROFILE_FIELD.to_owned(),
            message: format!("this profile has no {file} yet; launch it once"),
        });
    }

    let connection = Connection::open_with_flags(&path, flags).map_err(internal)?;
    connection.busy_timeout(BUSY_TIMEOUT).map_err(internal)?;

    Ok(connection)
}

// Firefox's integers for Cookie.sameSite.
fn same_site(value: i64) -> SameSite {
    match value {
        SAME_SITE_NONE | SAME_SITE_UNSET => SameSite::None,
        SAME_SITE_STRICT => SameSite::Strict,
        _ => SameSite::Lax,
    }
}

// None is written as unset, as Juggler does: an explicit 0 would be a stricter cookie than the
// one a running Instance reports.
fn same_site_value(same_site: SameSite) -> i64 {
    match same_site {
        SameSite::None => SAME_SITE_UNSET,
        SameSite::Lax => SAME_SITE_LAX,
        SameSite::Strict => SAME_SITE_STRICT,
    }
}

pub fn read_cookies_offline(profile_dir: &Path) -> Result<Vec<Cookie>, Error> {
    let connection = open(profile_dir, COOKIES_FILE, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // Containers and partitioned cookies carry origin attributes; they are not the Profile's own.
    let mut statement = connection
        .prepare(
            "SELECT host, name, value, path, expiry, isSecure, isHttpOnly, sameSite \
             FROM moz_cookies WHERE originAttributes = ''",
        )
        .map_err(internal)?;
    let rows = statement
        .query_map([], |row| {
            let expiry: i64 = row.get(4)?;

            Ok(Cookie {
                domain: row.get(0)?,
                name: row.get(1)?,
                value: row.get(2)?,
                path: row.get(3)?,
                expires: if expiry > 0 {
                    (expiry / MILLIS) as f64
                } else {
                    -1.0
                },
                secure: row.get(5)?,
                http_only: row.get(6)?,
                same_site: same_site(row.get(7)?),
            })
        })
        .map_err(internal)?;

    rows.collect::<Result<_, _>>().map_err(internal)
}

pub fn write_cookies_offline(profile_dir: &Path, cookies: &[Cookie]) -> Result<u32, Error> {
    let mut connection = open(profile_dir, COOKIES_FILE, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let now = now_micros();
    let transaction = connection.transaction().map_err(internal)?;

    for cookie in cookies {
        let expiry = if cookie.expires >= 0.0 {
            cookie.expires as i64
        } else {
            now / MICROS + SESSION_COOKIE_LIFETIME_S
        } * MILLIS;
        let updated = transaction
            .execute(
                "UPDATE moz_cookies SET value = ?1, expiry = ?2, lastAccessed = ?3, updateTime = ?3, \
                 isSecure = ?4, isHttpOnly = ?5, sameSite = ?6 \
                 WHERE name = ?7 AND host = ?8 AND path = ?9 AND originAttributes = ''",
                params![
                    cookie.value,
                    expiry,
                    now,
                    cookie.secure,
                    cookie.http_only,
                    same_site_value(cookie.same_site),
                    cookie.name,
                    cookie.domain,
                    cookie.path,
                ],
            )
            .map_err(internal)?;
        if updated == 0 {
            transaction
                .execute(
                    "INSERT INTO moz_cookies (originAttributes, name, value, host, path, expiry, \
                     lastAccessed, creationTime, updateTime, isSecure, isHttpOnly, sameSite) \
                     VALUES ('', ?1, ?2, ?3, ?4, ?5, ?6, ?6, ?6, ?7, ?8, ?9)",
                    params![
                        cookie.name,
                        cookie.value,
                        cookie.domain,
                        cookie.path,
                        expiry,
                        now,
                        cookie.secure,
                        cookie.http_only,
                        same_site_value(cookie.same_site),
                    ],
                )
                .map_err(internal)?;
        }
    }
    transaction.commit().map_err(internal)?;

    Ok(u32::try_from(cookies.len()).unwrap_or(u32::MAX))
}

struct Row {
    id: i64,
    kind: i64,
    title: String,
    url: Option<String>,
    added_micros: i64,
}

fn tree(children: &HashMap<i64, Vec<Row>>, parent: i64, depth: usize) -> Vec<Bookmark> {
    if depth > MAX_DEPTH {
        return Vec::new();
    }

    children
        .get(&parent)
        .into_iter()
        .flatten()
        .filter_map(|row| match (row.kind, &row.url) {
            (TYPE_BOOKMARK, Some(url)) if !url.starts_with(QUERY_SCHEME) => Some(Bookmark {
                title: row.title.clone(),
                url: Some(url.clone()),
                add_date: Some(row.added_micros / MICROS),
                children: Vec::new(),
            }),
            (TYPE_FOLDER, _) => Some(Bookmark {
                title: row.title.clone(),
                url: None,
                add_date: Some(row.added_micros / MICROS),
                children: tree(children, row.id, depth + 1),
            }),
            _ => None,
        })
        .collect()
}

pub fn export_bookmarks_offline(profile_dir: &Path) -> Result<String, Error> {
    let connection = open(profile_dir, PLACES_FILE, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = connection
        .prepare(
            "SELECT b.id, b.parent, b.type, COALESCE(b.title, ''), p.url, COALESCE(b.dateAdded, 0) \
             FROM moz_bookmarks b LEFT JOIN moz_places p ON p.id = b.fk \
             ORDER BY b.parent, b.position",
        )
        .map_err(internal)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(1)?,
                Row {
                    id: row.get(0)?,
                    kind: row.get(2)?,
                    title: row.get(3)?,
                    url: row.get(4)?,
                    added_micros: row.get(5)?,
                },
            ))
        })
        .map_err(internal)?;

    let mut children: HashMap<i64, Vec<Row>> = HashMap::new();
    for row in rows {
        let (parent, row) = row.map_err(internal)?;
        children.entry(parent).or_default().push(row);
    }

    let mut roots = Vec::new();
    for (guid, title) in ROOTS {
        let root_id: Option<i64> = connection
            .query_row(
                "SELECT id FROM moz_bookmarks WHERE guid = ?1",
                [guid],
                |row| row.get(0),
            )
            .optional()
            .map_err(internal)?;
        if let Some(root_id) = root_id {
            roots.push(Bookmark {
                title: title.to_owned(),
                url: None,
                add_date: None,
                children: tree(&children, root_id, 0),
            });
        }
    }

    Ok(bookmark::to_html(&roots))
}

fn guid() -> String {
    (0..GUID_LEN)
        .map(|_| char::from(GUID_ALPHABET[rand::random_range(0..GUID_ALPHABET.len())]))
        .collect()
}

// mozilla::HashString over bytes: Places' url_hash column indexes on it.
fn string_hash(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |hash, byte| {
        GOLDEN_RATIO.wrapping_mul(hash.rotate_left(5) ^ u32::from(*byte))
    })
}

// The scheme's 16-bit hash in the high half, the URL's hash below it; both over at most
// MAX_HASHED_BYTES, as Places does.
fn url_hash(url: &str) -> i64 {
    let bytes = &url.as_bytes()[..url.len().min(MAX_HASHED_BYTES)];
    let scan = &bytes[..bytes.len().min(SCHEME_SCAN_BYTES)];

    match scan.iter().position(|byte| *byte == b':') {
        Some(colon) => {
            i64::from(string_hash(&bytes[..colon]) & PREFIX_HASH_MASK) << 32
                | i64::from(string_hash(bytes))
        }
        None => i64::from(string_hash(bytes)),
    }
}

// Firefox indexes hosts reversed with a trailing dot (`com.example.`).
fn reversed_host(url: &str) -> String {
    let host = url.split_once("://").map_or("", |(_, rest)| {
        rest.split(['/', '?', '#']).next().unwrap_or("")
    });
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    let host = host.rsplit_once(':').map_or(host, |(host, _)| host);

    let mut reversed: String = host.chars().rev().collect();
    reversed.push('.');

    reversed
}

fn insert(
    connection: &Connection,
    parent_id: i64,
    bookmark: &Bookmark,
    now: i64,
) -> Result<(), Error> {
    let position: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM moz_bookmarks WHERE parent = ?1",
            [parent_id],
            |row| row.get(0),
        )
        .map_err(internal)?;
    let added = bookmark.add_date.map_or(now, |date| date * MICROS);

    let (kind, place_id) = match &bookmark.url {
        Some(url) => (
            TYPE_BOOKMARK,
            Some(place_id(connection, url, &bookmark.title)?),
        ),
        None => (TYPE_FOLDER, None),
    };
    connection
        .execute(
            "INSERT INTO moz_bookmarks (type, fk, parent, position, title, dateAdded, lastModified, guid) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![kind, place_id, parent_id, position, bookmark.title, added, now, guid()],
        )
        .map_err(internal)?;
    let id = connection.last_insert_rowid();

    for child in &bookmark.children {
        insert(connection, id, child, now)?;
    }

    Ok(())
}

// The place of a URL, created when the Profile never visited it. Its foreign count keeps
// Firefox from expiring it as an orphan.
fn place_id(connection: &Connection, url: &str, title: &str) -> Result<i64, Error> {
    let existing: Option<i64> = connection
        .query_row("SELECT id FROM moz_places WHERE url = ?1", [url], |row| {
            row.get(0)
        })
        .optional()
        .map_err(internal)?;
    if let Some(id) = existing {
        connection
            .execute(
                "UPDATE moz_places SET foreign_count = foreign_count + 1 WHERE id = ?1",
                [id],
            )
            .map_err(internal)?;

        return Ok(id);
    }

    connection
        .execute(
            "INSERT INTO moz_places (url, title, rev_host, hidden, frecency, guid, foreign_count, url_hash) \
             VALUES (?1, ?2, ?3, 0, -1, ?4, 1, ?5)",
            params![url, title, reversed_host(url), guid(), url_hash(url)],
        )
        .map_err(internal)?;

    Ok(connection.last_insert_rowid())
}

fn imported_folder(connection: &Connection, now: i64) -> Result<i64, Error> {
    let root_id: i64 = connection
        .query_row(
            "SELECT id FROM moz_bookmarks WHERE guid = ?1",
            [IMPORT_ROOT],
            |row| row.get(0),
        )
        .map_err(|_| Error::InvalidArgument {
            field: PROFILE_FIELD.to_owned(),
            message: "this profile has no bookmarks menu yet; launch it once".to_owned(),
        })?;
    let existing: Option<i64> = connection
        .query_row(
            "SELECT id FROM moz_bookmarks WHERE parent = ?1 AND type = ?2 AND title = ?3",
            params![root_id, TYPE_FOLDER, IMPORT_FOLDER],
            |row| row.get(0),
        )
        .optional()
        .map_err(internal)?;
    if let Some(id) = existing {
        return Ok(id);
    }

    let folder = Bookmark {
        title: IMPORT_FOLDER.to_owned(),
        url: None,
        add_date: None,
        children: Vec::new(),
    };
    insert(connection, root_id, &folder, now)?;

    Ok(connection.last_insert_rowid())
}

pub fn import_bookmarks_offline(profile_dir: &Path, html: &str) -> Result<u32, Error> {
    let roots = bookmark::from_html(html)?;
    let imported: Vec<&Bookmark> = roots
        .iter()
        .flat_map(|root| match root.url {
            Some(_) => std::slice::from_ref(root),
            None => root.children.as_slice(),
        })
        .collect();

    let mut connection = open(profile_dir, PLACES_FILE, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let now = now_micros();
    let transaction = connection.transaction().map_err(internal)?;
    let folder_id = imported_folder(&transaction, now)?;
    for bookmark in &imported {
        insert(&transaction, folder_id, bookmark, now)?;
    }
    transaction.commit().map_err(internal)?;

    Ok(imported
        .into_iter()
        .map(|bookmark| bookmark::count(std::slice::from_ref(bookmark)))
        .sum())
}
