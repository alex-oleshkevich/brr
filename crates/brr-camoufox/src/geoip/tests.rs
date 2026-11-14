use std::{fs, net::IpAddr};

use brr_core::Error;
use tempfile::TempDir;

use super::GeoDb;

const RECORD_BITS: u16 = 24;
const RECORD_BYTES: usize = 3;
const DATA_SEPARATOR_BYTES: usize = 16;
const METADATA_MARKER: &[u8] = b"\xab\xcd\xefMaxMind.com";
const COVERED: [u8; 3] = [1, 2, 3];
const IN_RANGE: &str = "1.2.3.4";
const OUT_OF_RANGE: &str = "9.9.9.9";
const COUNTRY: &str = "DE";
const TIMEZONE: &str = "Europe/Berlin";
const LATITUDE: f64 = 52.5;
const LONGITUDE: f64 = 13.4;
const LOCALE: &str = "de-DE";

// The MaxMind DB encoding of the few types the fixture uses
fn control(kind: u8, len: usize) -> Vec<u8> {
    if kind <= 7 {
        vec![(kind << 5) | len as u8]
    } else {
        vec![len as u8, kind - 7]
    }
}

fn string(text: &str) -> Vec<u8> {
    [control(2, text.len()), text.as_bytes().to_vec()].concat()
}

fn double(value: f64) -> Vec<u8> {
    [control(3, 8), value.to_be_bytes().to_vec()].concat()
}

fn uint(kind: u8, value: u64, width: usize) -> Vec<u8> {
    [
        control(kind, width),
        value.to_be_bytes()[8 - width..].to_vec(),
    ]
    .concat()
}

fn map(entries: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    let mut out = control(7, entries.len());
    for (key, value) in entries {
        out.extend(string(key));
        out.extend(value);
    }
    out
}

fn record() -> Vec<u8> {
    map(vec![
        ("country", map(vec![("iso_code", string(COUNTRY))])),
        (
            "location",
            map(vec![
                ("latitude", double(LATITUDE)),
                ("longitude", double(LONGITUDE)),
                ("time_zone", string(TIMEZONE)),
            ]),
        ),
    ])
}

// An IPv4 database whose only network is 1.2.3.0/24
fn database() -> Vec<u8> {
    let bits: Vec<usize> = COVERED
        .iter()
        .flat_map(|byte| {
            (0..8)
                .rev()
                .map(move |shift| usize::from(byte >> shift & 1))
        })
        .collect();
    let node_count = bits.len();
    let data = record();

    let mut tree = Vec::new();
    for (depth, bit) in bits.iter().enumerate() {
        let mut records = [node_count; 2];
        // A value past the node count points into the data section
        records[*bit] = if depth + 1 < node_count {
            depth + 1
        } else {
            node_count + DATA_SEPARATOR_BYTES
        };
        for value in records {
            tree.extend(&(value as u32).to_be_bytes()[4 - RECORD_BYTES..]);
        }
    }

    let metadata = map(vec![
        ("binary_format_major_version", uint(5, 2, 2)),
        ("binary_format_minor_version", uint(5, 0, 2)),
        ("build_epoch", uint(9, 0, 4)),
        ("database_type", string("Test")),
        ("description", map(vec![("en", string("Test"))])),
        ("ip_version", uint(5, 4, 2)),
        ("languages", [control(11, 1), string("en")].concat()),
        ("node_count", uint(6, node_count as u64, 4)),
        ("record_size", uint(5, u64::from(RECORD_BITS), 2)),
    ]);

    [
        tree,
        vec![0; DATA_SEPARATOR_BYTES],
        data,
        METADATA_MARKER.to_vec(),
        metadata,
    ]
    .concat()
}

fn open(bytes: &[u8]) -> Result<GeoDb, Error> {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("geoip.mmdb");
    fs::write(&path, bytes).unwrap();
    GeoDb::open(&path)
}

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

#[test]
fn lookup_reads_the_record_of_a_covered_address() {
    let geo = open(&database()).unwrap().lookup(ip(IN_RANGE)).unwrap();

    assert_eq!(geo.country, COUNTRY);
    assert_eq!(geo.timezone, TIMEZONE);
    assert_eq!(geo.latitude, LATITUDE);
    assert_eq!(geo.longitude, LONGITUDE);
    assert_eq!(geo.locale, LOCALE);
    assert_eq!(geo.ip, ip(IN_RANGE));
}

#[test]
fn lookup_refuses_an_address_outside_every_network() {
    let err = open(&database())
        .unwrap()
        .lookup(ip(OUT_OF_RANGE))
        .unwrap_err();

    assert_eq!(
        err,
        Error::InvalidArgument {
            field: "geoip".to_owned(),
            message: format!("{OUT_OF_RANGE} is not in the GeoIP database"),
        }
    );
}

#[test]
fn a_truncated_database_is_refused() {
    let bytes = database();

    for kept in [0, bytes.len() / 2, bytes.len() - 1] {
        let err = open(&bytes[..kept]).err().unwrap();

        assert!(
            matches!(err, Error::InvalidArgument { ref field, .. } if field == "geoip"),
            "{kept} bytes kept: {err:?}"
        );
    }
}

#[test]
fn a_missing_file_says_to_install_the_database() {
    let dir = TempDir::new().unwrap();

    let err = GeoDb::open(&dir.path().join("absent.mmdb")).err().unwrap();

    assert!(err.to_string().contains("install it"), "{err}");
}
