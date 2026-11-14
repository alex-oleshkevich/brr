use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aes::Aes128;
use brr_core::{Bookmark, Cookie, Error, SameSite, bookmark};
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OpenFlags, params, params_from_iter};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tracing::warn;

use super::{internal, write_atomic};

#[cfg(test)]
mod tests;

// PBKDF2-HMAC-SHA1("peanuts", "saltysalt", 1 iteration, 16 bytes): the fixed key Chromium on
// Linux derives when started with --password-store=basic. A public constant, not a secret.
const V10_KEY: [u8; 16] = [
    0xfd, 0x62, 0x1f, 0xe5, 0xa2, 0xb4, 0x02, 0x53, 0x9d, 0xfa, 0x14, 0x7c, 0xa9, 0x27, 0x27, 0x78,
];
const V10_IV: [u8; 16] = [0x20; 16];
const V10_PREFIX: &[u8] = b"v10";
const HASHED_HOST_VERSION: i64 = 24;
const HASH_LEN: usize = 32;
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);
const WEBKIT_EPOCH_OFFSET_S: i64 = 11_644_473_600;
const MICROS: i64 = 1_000_000;
const PROFILE_FIELD: &str = "profile";
const KNOWN: [&str; 15] = [
    "creation_utc",
    "host_key",
    "name",
    "value",
    "encrypted_value",
    "path",
    "expires_utc",
    "is_secure",
    "is_httponly",
    "last_access_utc",
    "has_expires",
    "is_persistent",
    "samesite",
    "last_update_utc",
    "priority",
];

const BAR_ROOT: (&str, &str) = ("bookmark_bar", "Bookmarks bar");
const OTHER_ROOT: (&str, &str) = ("other", "Other bookmarks");
const SYNCED_ROOT: (&str, &str) = ("synced", "Mobile bookmarks");

fn now_micros() -> i64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    i64::try_from(since_epoch.as_micros()).unwrap_or(i64::MAX)
}

fn webkit_micros(unix_s: f64) -> i64 {
    (unix_s * MICROS as f64) as i64 + WEBKIT_EPOCH_OFFSET_S * MICROS
}

fn unix_secs(webkit_micros: i64) -> i64 {
    webkit_micros / MICROS - WEBKIT_EPOCH_OFFSET_S
}

fn open_cookies(user_data_dir: &Path) -> Result<Connection, Error> {
    let path = user_data_dir.join("Default").join("Cookies");
    if !path.exists() {
        return Err(Error::InvalidArgument {
            field: PROFILE_FIELD.to_owned(),
            message: "this profile has no cookie store yet; launch it once".to_owned(),
        });
    }

    let connection =
        Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(internal)?;
    connection.busy_timeout(BUSY_TIMEOUT).map_err(internal)?;

    Ok(connection)
}

fn meta_version(connection: &Connection) -> Result<i64, Error> {
    connection
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'version'",
            [],
            |row| row.get(0),
        )
        .map_err(internal)
}

fn host_hash(host_key: &str) -> [u8; HASH_LEN] {
    Sha256::digest(host_key.as_bytes()).into()
}

fn decrypt(host_key: &str, blob: &[u8], version: i64) -> Option<String> {
    let ciphertext = blob.strip_prefix(V10_PREFIX)?;
    let plain = cbc::Decryptor::<Aes128>::new(&V10_KEY.into(), &V10_IV.into())
        .decrypt_padded_vec::<Pkcs7>(ciphertext)
        .ok()?;

    let value = if version >= HASHED_HOST_VERSION {
        plain.strip_prefix(&host_hash(host_key))?
    } else {
        &plain
    };

    String::from_utf8(value.to_vec()).ok()
}

fn encrypt(host_key: &str, value: &str, version: i64) -> Vec<u8> {
    let mut plain = Vec::new();
    if version >= HASHED_HOST_VERSION {
        plain.extend_from_slice(&host_hash(host_key));
    }
    plain.extend_from_slice(value.as_bytes());

    let mut blob = V10_PREFIX.to_vec();
    blob.extend(
        cbc::Encryptor::<Aes128>::new(&V10_KEY.into(), &V10_IV.into())
            .encrypt_padded_vec::<Pkcs7>(&plain),
    );

    blob
}

pub fn read_cookies(user_data_dir: &Path) -> Result<Vec<Cookie>, Error> {
    let connection = open_cookies(user_data_dir)?;
    let version = meta_version(&connection)?;

    let mut statement = connection
        .prepare(
            "SELECT host_key, name, value, encrypted_value, path, expires_utc, is_secure, \
             is_httponly, has_expires, samesite FROM cookies",
        )
        .map_err(internal)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, bool>(7)?,
                row.get::<_, bool>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })
        .map_err(internal)?;

    let mut cookies = Vec::new();
    let mut skipped = 0_u32;

    for row in rows {
        let (
            domain,
            name,
            plain,
            encrypted,
            path,
            expires_utc,
            secure,
            http_only,
            has_expires,
            same_site,
        ) = row.map_err(internal)?;

        let value = if encrypted.is_empty() {
            plain
        } else if let Some(value) = decrypt(&domain, &encrypted, version) {
            value
        } else {
            skipped += 1;
            continue;
        };

        cookies.push(Cookie {
            name,
            value,
            domain,
            path,
            expires: if has_expires && expires_utc != 0 {
                unix_secs(expires_utc) as f64
            } else {
                -1.0
            },
            http_only,
            secure,
            same_site: match same_site {
                0 => SameSite::None,
                2 => SameSite::Strict,
                _ => SameSite::Lax,
            },
        });
    }

    if skipped > 0 {
        warn!(skipped, "skipped cookies that could not be decrypted");
    }

    Ok(cookies)
}

pub fn write_cookies(user_data_dir: &Path, cookies: &[Cookie]) -> Result<u32, Error> {
    let mut connection = open_cookies(user_data_dir)?;
    let version = meta_version(&connection)?;
    let columns = cookie_columns(&connection)?;
    let now = now_micros() + WEBKIT_EPOCH_OFFSET_S * MICROS;

    let sql = format!(
        "INSERT INTO cookies ({}) VALUES ({})",
        columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        vec!["?"; columns.len()].join(", ")
    );

    let transaction = connection.transaction().map_err(internal)?;
    for cookie in cookies {
        transaction
            .execute(
                "DELETE FROM cookies WHERE host_key = ?1 AND name = ?2 AND path = ?3",
                params![cookie.domain, cookie.name, cookie.path],
            )
            .map_err(internal)?;

        let persistent = cookie.expires >= 0.0;
        let values = columns.iter().map(|column| match column.name.as_str() {
            "creation_utc" | "last_access_utc" | "last_update_utc" => Sql::Integer(now),
            "host_key" => Sql::Text(cookie.domain.clone()),
            "name" => Sql::Text(cookie.name.clone()),
            "value" => Sql::Text(String::new()),
            "encrypted_value" => Sql::Blob(encrypt(&cookie.domain, &cookie.value, version)),
            "path" => Sql::Text(cookie.path.clone()),
            "expires_utc" => Sql::Integer(if persistent {
                webkit_micros(cookie.expires)
            } else {
                0
            }),
            "is_secure" => Sql::Integer(cookie.secure.into()),
            "is_httponly" => Sql::Integer(cookie.http_only.into()),
            "has_expires" | "is_persistent" => Sql::Integer(persistent.into()),
            // Medium, Chromium's default priority
            "priority" => Sql::Integer(1),
            "samesite" => Sql::Integer(match cookie.same_site {
                SameSite::None => 0,
                SameSite::Lax => 1,
                SameSite::Strict => 2,
            }),
            _ if column.integer => Sql::Integer(0),
            _ => Sql::Text(String::new()),
        });
        transaction
            .execute(&sql, params_from_iter(values))
            .map_err(internal)?;
    }
    transaction.commit().map_err(internal)?;

    Ok(u32::try_from(cookies.len()).unwrap_or(u32::MAX))
}

struct Column {
    name: String,
    integer: bool,
}

// Chromium adds cookie columns across versions. Columns with a default are left to it unless
// brr has a value for them.
fn cookie_columns(connection: &Connection) -> Result<Vec<Column>, Error> {
    let mut statement = connection
        .prepare(
            "SELECT name, type, \"notnull\", dflt_value IS NULL FROM pragma_table_info('cookies')",
        )
        .map_err(internal)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })
        .map_err(internal)?;

    let mut columns = Vec::new();
    for row in rows {
        let (name, kind, not_null, no_default) = row.map_err(internal)?;
        if KNOWN.contains(&name.as_str()) || (not_null && no_default) {
            columns.push(Column {
                name,
                integer: kind.to_ascii_uppercase().contains("INT"),
            });
        }
    }

    Ok(columns)
}

fn bookmarks_path(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("Default").join("Bookmarks")
}

fn load_bookmarks(user_data_dir: &Path) -> Result<Option<Value>, Error> {
    let path = bookmarks_path(user_data_dir);

    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| internal(format!("{} is not valid JSON: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(internal(format!("{}: {error}", path.display()))),
    }
}

pub fn read_bookmarks(user_data_dir: &Path) -> Result<Vec<Bookmark>, Error> {
    let Some(file) = load_bookmarks(user_data_dir)? else {
        return Ok(Vec::new());
    };

    let mut roots = Vec::new();
    for (key, title) in [BAR_ROOT, OTHER_ROOT, SYNCED_ROOT] {
        let Some(root) = file["roots"].get(key) else {
            continue;
        };
        let children = children_of(root);
        if key == SYNCED_ROOT.0 && children.is_empty() {
            continue;
        }

        roots.push(Bookmark {
            title: title.to_owned(),
            url: None,
            add_date: None,
            children,
        });
    }

    Ok(roots)
}

fn children_of(node: &Value) -> Vec<Bookmark> {
    node["children"]
        .as_array()
        .map(|children| children.iter().map(to_bookmark).collect())
        .unwrap_or_default()
}

fn to_bookmark(node: &Value) -> Bookmark {
    Bookmark {
        title: node["name"].as_str().unwrap_or_default().to_owned(),
        url: node["url"].as_str().map(str::to_owned),
        add_date: node["date_added"]
            .as_str()
            .and_then(|date| date.parse().ok())
            .map(unix_secs),
        children: children_of(node),
    }
}

pub fn add_bookmarks(user_data_dir: &Path, roots: &[Bookmark]) -> Result<u32, Error> {
    let mut file = load_bookmarks(user_data_dir)?.unwrap_or_else(empty_bookmarks);

    let mut next_id = max_id(&file["roots"]) + 1;
    let now = now_micros() + WEBKIT_EPOCH_OFFSET_S * MICROS;
    let appended: Vec<&Bookmark> = roots
        .iter()
        .flat_map(|root| match root.url {
            Some(_) => std::slice::from_ref(root),
            None => root.children.as_slice(),
        })
        .collect();
    let nodes: Vec<Value> = appended
        .iter()
        .map(|bookmark| to_node(bookmark, &mut next_id, now))
        .collect();

    let Some(children) = file
        .pointer_mut("/roots/bookmark_bar/children")
        .and_then(Value::as_array_mut)
    else {
        return Err(internal("the Bookmarks file has no bookmark_bar root"));
    };
    children.extend(nodes);

    // The checksum is removed, never verified on load; Chromium writes a fresh one on save.
    if let Some(top) = file.as_object_mut() {
        top.remove("checksum");
    }
    let bytes = serde_json::to_vec(&file).map_err(internal)?;
    write_atomic(&bookmarks_path(user_data_dir), &bytes)?;

    Ok(appended
        .into_iter()
        .map(|bookmark| bookmark::count(std::slice::from_ref(bookmark)))
        .sum())
}

fn empty_bookmarks() -> Value {
    let mut roots = Map::new();
    for (index, (key, title)) in [BAR_ROOT, OTHER_ROOT, SYNCED_ROOT].into_iter().enumerate() {
        roots.insert(
            key.to_owned(),
            json!({
                "children": [],
                "date_added": "0",
                "id": (index + 1).to_string(),
                "name": title,
                "type": "folder",
            }),
        );
    }

    json!({ "roots": roots, "version": 1 })
}

fn max_id(node: &Value) -> u64 {
    let own = node["id"].as_str().and_then(|id| id.parse().ok());
    let nested = match node {
        Value::Object(map) => map.values().map(max_id).max(),
        Value::Array(items) => items.iter().map(max_id).max(),
        _ => None,
    };

    own.max(nested).unwrap_or(0)
}

fn to_node(bookmark: &Bookmark, next_id: &mut u64, now: i64) -> Value {
    let id = *next_id;
    *next_id += 1;

    let mut node = json!({
        "date_added": now.to_string(),
        "date_last_used": "0",
        "guid": uuid_v4(),
        "id": id.to_string(),
        "name": bookmark.title,
    });

    match &bookmark.url {
        Some(url) => {
            node["type"] = json!("url");
            node["url"] = json!(url);
        }
        None => {
            node["type"] = json!("folder");
            node["children"] = bookmark
                .children
                .iter()
                .map(|child| to_node(child, next_id, now))
                .collect();
        }
    }

    node
}

fn uuid_v4() -> String {
    let bits = (rand::random::<u128>() & !(0xf << 76) & !(0x3 << 62)) | (0x4 << 76) | (0x2 << 62);
    let hex = format!("{bits:032x}");

    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
