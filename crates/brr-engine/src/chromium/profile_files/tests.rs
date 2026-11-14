#![allow(clippy::unwrap_used)]

use std::path::Path;

use aes::Aes128;
use brr_core::{Bookmark, Cookie, Error, SameSite, bookmark};
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeEncrypt, KeyIvInit};
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::{
    V10_IV, V10_KEY, V10_PREFIX, add_bookmarks, read_bookmarks, read_cookies, write_cookies,
};

const HOST: &str = ".example.test";
// 2030-01-01T00:00:00Z
const EXPIRES: f64 = 1_893_456_000.0;

fn profile() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("Default")).unwrap();

    dir
}

fn cookies_db(dir: &Path) -> Connection {
    let connection = Connection::open(dir.join("Default").join("Cookies")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE meta (key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR);
             INSERT INTO meta VALUES ('version', '24');
             CREATE TABLE cookies (
                creation_utc INTEGER NOT NULL,
                host_key TEXT NOT NULL,
                top_frame_site_key TEXT NOT NULL,
                name TEXT NOT NULL,
                value TEXT NOT NULL,
                encrypted_value BLOB NOT NULL,
                path TEXT NOT NULL,
                expires_utc INTEGER NOT NULL,
                is_secure INTEGER NOT NULL,
                is_httponly INTEGER NOT NULL,
                last_access_utc INTEGER NOT NULL,
                has_expires INTEGER NOT NULL,
                is_persistent INTEGER NOT NULL,
                priority INTEGER NOT NULL,
                samesite INTEGER NOT NULL,
                source_scheme INTEGER NOT NULL,
                source_port INTEGER NOT NULL,
                last_update_utc INTEGER NOT NULL
             );",
        )
        .unwrap();

    connection
}

fn cookie(name: &str, value: &str, expires: f64) -> Cookie {
    Cookie {
        name: name.to_owned(),
        value: value.to_owned(),
        domain: HOST.to_owned(),
        path: "/".to_owned(),
        expires,
        http_only: true,
        secure: true,
        same_site: SameSite::Strict,
    }
}

#[test]
fn cookies_roundtrip() {
    let dir = profile();
    let connection = cookies_db(dir.path());

    let mut plain = Sha256::digest(HOST.as_bytes()).to_vec();
    plain.extend_from_slice(b"fixture-secret");
    let mut blob = V10_PREFIX.to_vec();
    blob.extend(
        cbc::Encryptor::<Aes128>::new(&V10_KEY.into(), &V10_IV.into())
            .encrypt_padded_vec::<Pkcs7>(&plain),
    );
    connection
        .execute(
            "INSERT INTO cookies VALUES (1, ?1, '', 'fixture', '', ?2, '/', 0, 0, 0, 1, 0, 0, 1, -1, 0, -1, 1)",
            rusqlite::params![HOST, blob],
        )
        .unwrap();
    drop(connection);

    let read = read_cookies(dir.path()).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].value, "fixture-secret");
    assert_eq!(read[0].expires, -1.0);
    assert_eq!(read[0].same_site, SameSite::Lax);

    let written = [
        cookie("one", "first", EXPIRES),
        cookie("two", "second", -1.0),
    ];
    assert_eq!(write_cookies(dir.path(), &written).unwrap(), 2);

    let mut read = read_cookies(dir.path()).unwrap();
    read.retain(|cookie| cookie.name != "fixture");
    read.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(read, written);

    let connection = Connection::open(dir.path().join("Default").join("Cookies")).unwrap();
    let blobs: Vec<Vec<u8>> = connection
        .prepare("SELECT encrypted_value FROM cookies WHERE name IN ('one', 'two')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(blobs.len(), 2);
    assert!(blobs.iter().all(|blob| blob.starts_with(V10_PREFIX)));

    write_cookies(dir.path(), &[cookie("one", "replaced", EXPIRES)]).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM cookies WHERE name = 'one'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn cookies_missing_store() {
    let dir = profile();

    let error = read_cookies(dir.path()).unwrap_err();

    assert!(matches!(error, Error::InvalidArgument { field, .. } if field == "profile"));
}

fn link(title: &str, url: &str) -> Bookmark {
    Bookmark {
        title: title.to_owned(),
        url: Some(url.to_owned()),
        add_date: None,
        children: Vec::new(),
    }
}

#[test]
fn bookmarks_add() {
    let dir = profile();
    let existing = json!({
        "checksum": "stale",
        "roots": {
            "bookmark_bar": {"id": "1", "name": "Bookmarks bar", "type": "folder", "children": [
                {"id": "5", "name": "Folder", "type": "folder", "date_added": "13000000000000000", "children": []},
                {"id": "6", "name": "Link", "type": "url", "url": "https://a.test/", "date_added": "13000000000000000"}
            ]},
            "other": {"id": "2", "name": "Other bookmarks", "type": "folder", "children": []},
            "synced": {"id": "3", "name": "Mobile bookmarks", "type": "folder", "children": []}
        },
        "version": 1
    });
    std::fs::write(
        dir.path().join("Default").join("Bookmarks"),
        existing.to_string(),
    )
    .unwrap();

    let imported = [Bookmark {
        title: "Bookmarks bar".to_owned(),
        url: None,
        add_date: None,
        children: vec![link("B", "https://b.test/"), link("C", "https://c.test/")],
    }];
    assert_eq!(add_bookmarks(dir.path(), &imported).unwrap(), 2);

    let roots = read_bookmarks(dir.path()).unwrap();
    assert_eq!(roots[0].title, "Bookmarks bar");
    assert_eq!(bookmark::count(&roots[0..1]), 3);

    let file: Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("Default").join("Bookmarks")).unwrap(),
    )
    .unwrap();
    assert!(file.get("checksum").is_none());

    let mut ids = Vec::new();
    collect_ids(&file["roots"], &mut ids);
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len());
    assert_eq!(ids.len(), 7);
}

fn collect_ids(node: &Value, ids: &mut Vec<String>) {
    match node {
        Value::Object(map) => {
            if let Some(id) = map.get("id").and_then(Value::as_str) {
                ids.push(id.to_owned());
            }
            map.values().for_each(|child| collect_ids(child, ids));
        }
        Value::Array(items) => items.iter().for_each(|child| collect_ids(child, ids)),
        _ => {}
    }
}

#[test]
fn bookmarks_absent_file_is_empty_then_created() {
    let dir = profile();

    assert!(read_bookmarks(dir.path()).unwrap().is_empty());

    add_bookmarks(
        dir.path(),
        &[Bookmark {
            title: "x".to_owned(),
            url: None,
            add_date: None,
            children: vec![link("A", "https://a.test/")],
        }],
    )
    .unwrap();

    assert_eq!(bookmark::count(&read_bookmarks(dir.path()).unwrap()), 1);
}

#[test]
fn bookmarks_add_keeps_top_level_links() {
    let dir = profile();
    let folder = Bookmark {
        title: "Imported".to_owned(),
        url: None,
        add_date: None,
        children: vec![link("B", "https://b.test/")],
    };

    let added = add_bookmarks(
        dir.path(),
        &[
            link("A", "https://a.test/"),
            folder,
            link("C", "https://c.test/"),
        ],
    )
    .unwrap();

    assert_eq!(added, 3);
    let roots = read_bookmarks(dir.path()).unwrap();
    assert_eq!(bookmark::count(&roots[0..1]), 3);
}
