#![allow(clippy::unwrap_used)]

use std::path::Path;

use brr_core::{Bookmark, Cookie, Error, SameSite, bookmark};
use rusqlite::Connection;
use tempfile::TempDir;

use super::{
    export_bookmarks_offline, import_bookmarks_offline, read_cookies_offline, url_hash,
    write_cookies_offline,
};

const HOST: &str = ".example.test";
// 2030-01-01T00:00:00Z
const EXPIRES: f64 = 1_893_456_000.0;

// The tables as Firefox 158 writes them, with the columns brr uses plus the ones it leaves alone.
fn cookies_db(dir: &Path) -> Connection {
    let connection = Connection::open(dir.join("cookies.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE moz_cookies (id INTEGER PRIMARY KEY, originAttributes TEXT NOT NULL DEFAULT '', name TEXT, value TEXT, host TEXT, path TEXT, expiry INTEGER, lastAccessed INTEGER, creationTime INTEGER, isSecure INTEGER, isHttpOnly INTEGER, inBrowserElement INTEGER DEFAULT 0, sameSite INTEGER DEFAULT 0, schemeMap INTEGER DEFAULT 0, isPartitionedAttributeSet INTEGER DEFAULT 0, updateTime INTEGER, CONSTRAINT moz_uniqueid UNIQUE (name, host, path, originAttributes));",
        )
        .unwrap();

    connection
}

fn places_db(dir: &Path) -> Connection {
    let connection = Connection::open(dir.join("places.sqlite")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url LONGVARCHAR, title LONGVARCHAR, rev_host LONGVARCHAR, visit_count INTEGER DEFAULT 0, hidden INTEGER DEFAULT 0 NOT NULL, typed INTEGER DEFAULT 0 NOT NULL, frecency INTEGER DEFAULT -1 NOT NULL, last_visit_date INTEGER, guid TEXT, foreign_count INTEGER DEFAULT 0 NOT NULL, url_hash INTEGER DEFAULT 0 NOT NULL);
             CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER DEFAULT NULL, parent INTEGER, position INTEGER, title LONGVARCHAR, keyword_id INTEGER, folder_type TEXT, dateAdded INTEGER, lastModified INTEGER, guid TEXT, syncStatus INTEGER NOT NULL DEFAULT 0, syncChangeCounter INTEGER NOT NULL DEFAULT 1);
             CREATE UNIQUE INDEX moz_bookmarks_guid_uniqueindex ON moz_bookmarks (guid);
             INSERT INTO moz_bookmarks (id, type, parent, position, title, guid) VALUES
                (1, 2, 0, 0, '', 'root________'),
                (2, 2, 1, 0, 'menu', 'menu________'),
                (3, 2, 1, 1, 'toolbar', 'toolbar_____'),
                (4, 2, 1, 2, 'tags', 'tags________'),
                (5, 2, 1, 3, 'unfiled', 'unfiled_____'),
                (6, 2, 1, 4, 'mobile', 'mobile______');",
        )
        .unwrap();

    connection
}

fn cookie(name: &str, same_site: SameSite, expires: f64) -> Cookie {
    Cookie {
        name: name.to_owned(),
        value: format!("{name}-value"),
        domain: HOST.to_owned(),
        path: "/".to_owned(),
        expires,
        http_only: true,
        secure: true,
        same_site,
    }
}

#[test]
fn firefox_same_site_integers_map_to_their_names() {
    let dir = TempDir::new().unwrap();
    let connection = cookies_db(dir.path());
    for (name, value) in [("none", 0), ("lax", 1), ("strict", 2), ("unset", 256)] {
        connection
            .execute(
                "INSERT INTO moz_cookies (name, value, host, path, expiry, isSecure, isHttpOnly, sameSite) \
                 VALUES (?1, 'v', ?2, '/', ?3, 0, 0, ?4)",
                rusqlite::params![name, HOST, EXPIRES as i64 * 1000, value],
            )
            .unwrap();
    }
    drop(connection);

    let mut read = read_cookies_offline(dir.path()).unwrap();
    read.sort_by(|a, b| a.name.cmp(&b.name));
    let same_sites: Vec<_> = read
        .iter()
        .map(|cookie| (cookie.name.as_str(), cookie.same_site))
        .collect();

    assert_eq!(
        same_sites,
        [
            ("lax", SameSite::Lax),
            ("none", SameSite::None),
            ("strict", SameSite::Strict),
            ("unset", SameSite::None),
        ]
    );
}

#[test]
fn cookies_roundtrip_every_same_site() {
    let dir = TempDir::new().unwrap();
    cookies_db(dir.path());
    let written = [
        cookie("lax", SameSite::Lax, EXPIRES),
        cookie("none", SameSite::None, EXPIRES),
        cookie("strict", SameSite::Strict, EXPIRES),
    ];

    assert_eq!(write_cookies_offline(dir.path(), &written).unwrap(), 3);

    let mut read = read_cookies_offline(dir.path()).unwrap();
    read.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(read, written);
}

#[test]
fn writing_a_cookie_again_replaces_it() {
    let dir = TempDir::new().unwrap();
    cookies_db(dir.path());
    write_cookies_offline(dir.path(), &[cookie("same", SameSite::Lax, EXPIRES)]).unwrap();

    let mut changed = cookie("same", SameSite::Strict, EXPIRES);
    changed.value = "changed".to_owned();
    write_cookies_offline(dir.path(), std::slice::from_ref(&changed)).unwrap();

    assert_eq!(read_cookies_offline(dir.path()).unwrap(), [changed]);
}

#[test]
fn a_session_cookie_is_given_an_expiry() {
    let dir = TempDir::new().unwrap();
    cookies_db(dir.path());
    write_cookies_offline(dir.path(), &[cookie("session", SameSite::Lax, -1.0)]).unwrap();

    let read = read_cookies_offline(dir.path()).unwrap();

    assert!(read[0].expires > 0.0);
}

#[test]
fn a_profile_without_a_cookie_store_says_to_launch_it() {
    let dir = TempDir::new().unwrap();

    let error = read_cookies_offline(dir.path()).unwrap_err();

    assert!(matches!(error, Error::InvalidArgument { .. }), "{error:?}");
}

fn flatten(bookmarks: &[Bookmark], out: &mut Vec<(String, String)>) {
    for bookmark in bookmarks {
        if let Some(url) = &bookmark.url {
            out.push((bookmark.title.clone(), url.clone()));
        }
        flatten(&bookmark.children, out);
    }
}

fn links(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    flatten(&bookmark::from_html(html).unwrap(), &mut out);
    out.sort();

    out
}

fn add_bookmark(connection: &Connection, parent: i64, title: &str, url: Option<&str>) -> i64 {
    let place_id = url.map(|url| {
        connection
            .execute(
                "INSERT INTO moz_places (url, title, guid) VALUES (?1, ?2, ?1)",
                rusqlite::params![url, title],
            )
            .unwrap();
        connection.last_insert_rowid()
    });
    connection
        .execute(
            "INSERT INTO moz_bookmarks (type, fk, parent, position, title, dateAdded, guid) \
             VALUES (?1, ?2, ?3, 0, ?4, 1700000000000000, ?5)",
            rusqlite::params![
                if url.is_some() { 1 } else { 2 },
                place_id,
                parent,
                title,
                format!("g{title}")
            ],
        )
        .unwrap();

    connection.last_insert_rowid()
}

#[test]
fn bookmarks_export_import_export_keeps_titles_and_urls() {
    let source = TempDir::new().unwrap();
    let connection = places_db(source.path());
    add_bookmark(&connection, 2, "Menu link", Some("https://menu.example/"));
    let folder = add_bookmark(&connection, 3, "Reading", None);
    add_bookmark(
        &connection,
        folder,
        "Nested <&> link",
        Some("https://nested.example/a?b=1&c=2"),
    );
    add_bookmark(
        &connection,
        5,
        "Unfiled link",
        Some("https://unfiled.example/"),
    );
    add_bookmark(&connection, 5, "Smart", Some("place:sort=8&maxResults=10"));
    drop(connection);

    let exported = export_bookmarks_offline(source.path()).unwrap();
    let expected = links(&exported);
    assert_eq!(expected.len(), 3);

    let target = TempDir::new().unwrap();
    places_db(target.path());
    assert_eq!(
        import_bookmarks_offline(target.path(), &exported).unwrap(),
        3
    );

    assert_eq!(
        links(&export_bookmarks_offline(target.path()).unwrap()),
        expected
    );
}

#[test]
fn importing_twice_reuses_the_imported_folder() {
    let target = TempDir::new().unwrap();
    let connection = places_db(target.path());
    add_bookmark(&connection, 2, "Menu link", Some("https://menu.example/"));
    drop(connection);
    let html = bookmark::to_html(&[Bookmark {
        title: "root".to_owned(),
        url: None,
        add_date: None,
        children: vec![Bookmark {
            title: "Shared".to_owned(),
            url: Some("https://menu.example/".to_owned()),
            add_date: Some(1_700_000_000),
            children: Vec::new(),
        }],
    }]);

    import_bookmarks_offline(target.path(), &html).unwrap();
    import_bookmarks_offline(target.path(), &html).unwrap();

    let connection = Connection::open(target.path().join("places.sqlite")).unwrap();
    let folders: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM moz_bookmarks WHERE title = 'Imported'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let places: i64 = connection
        .query_row("SELECT COUNT(*) FROM moz_places", [], |row| row.get(0))
        .unwrap();
    assert_eq!((folders, places), (1, 1));
}

#[test]
fn none_is_written_as_unset() {
    let dir = TempDir::new().unwrap();
    let connection = cookies_db(dir.path());
    write_cookies_offline(dir.path(), &[cookie("none", SameSite::None, EXPIRES)]).unwrap();

    let stored: i64 = connection
        .query_row("SELECT sameSite FROM moz_cookies", [], |row| row.get(0))
        .unwrap();

    assert_eq!(stored, 256);
}

#[test]
fn only_the_first_1500_bytes_of_a_url_are_hashed() {
    let long = format!("https://example.com/{}", "a".repeat(1600));
    let cut = &long[..1500];

    assert_eq!(url_hash(&long), url_hash(cut));
}
