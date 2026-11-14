use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use brr_core::{Error, sha256_hex};
use serde_json::{Number, Value, json};
use tempfile::TempDir;
use test_case::test_case;
use zip::{ZipWriter, write::SimpleFileOptions};

use super::{
    Conditions, Constraint, generate,
    model::{Model, Ordered, parse_ordered, unpack_model},
};
use crate::{data::FpgenPin, pyrandom::PyRandom};

const MODEL_DIR_ENV: &str = "BRR_TEST_FPGEN_MODEL";
const STAMP_FILE: &str = ".pinned-model";
const NETWORK_JSON: &str = r#"{"nodes":[{"name":"zeta","parentNames":[],"possibleValues":["0R","0"],"conditionalProbabilities":{"0R":0.5,"0":0.5}},{"name":"alpha","parentNames":["zeta"],"possibleValues":["0"],"conditionalProbabilities":{"0R":{"0":1.0},"0":{"0":1.0}}}]}"#;
const VALUES_INDEX_JSON: &str = r#"{"0":4,"4":3}"#;
const VALUES_DAT: &[u8] = b"\"ab\"{}1";
const FIRST_VALUE: &str = "\"ab\"";
const SECOND_VALUE: &str = "{}1";
// base64.b85encode(bytes([1])) and bytes([255])
const SECOND_ID: &str = "0R";
const OUT_OF_RANGE_ID: &str = "{{";
const NETWORK_ZST: &str = "fingerprint-network.json.zst";
const VALUES_JSON_ZST: &str = "values.json.zst";
const VALUES_DAT_ZST: &str = "values.dat.zst";
const VALUES_DAT_NAME: &str = "values.dat";

fn compress(bytes: &[u8]) -> Vec<u8> {
    zstd::encode_all(bytes, 0).unwrap()
}

struct Archive {
    zip_path: PathBuf,
    pin: FpgenPin,
    _scratch: TempDir,
}

fn archive() -> Archive {
    let scratch = TempDir::new().unwrap();
    let members = [
        (NETWORK_ZST, compress(NETWORK_JSON.as_bytes())),
        (VALUES_JSON_ZST, compress(VALUES_INDEX_JSON.as_bytes())),
        (VALUES_DAT_ZST, compress(VALUES_DAT)),
    ];

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in &members {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    let zip_bytes = writer.finish().unwrap().into_inner();
    let zip_path = scratch.path().join("model-release.zip");
    fs::write(&zip_path, &zip_bytes).unwrap();

    let pin = FpgenPin {
        url: String::new(),
        size: zip_bytes.len() as u64,
        sha256: sha256_hex(&zip_bytes),
        files: members.iter().map(|(name, _)| (*name).to_owned()).collect(),
        file_sha256: members
            .iter()
            .map(|(name, bytes)| ((*name).to_owned(), sha256_hex(bytes)))
            .collect(),
        decompressed_sha256: BTreeMap::from([(VALUES_DAT_NAME.to_owned(), sha256_hex(VALUES_DAT))]),
    };
    Archive {
        zip_path,
        pin,
        _scratch: scratch,
    }
}

fn is_sha_mismatch(err: &Error) -> bool {
    matches!(err, Error::InvalidArgument { field, message } if field == "fpgen_model" && message.contains("sha256 mismatch"))
}

#[test]
fn unpack_verifies() {
    let Archive {
        zip_path,
        pin,
        _scratch,
    } = archive();
    let dir = TempDir::new().unwrap();

    unpack_model(&zip_path, &pin, dir.path()).unwrap();

    assert_eq!(
        fs::read(dir.path().join(VALUES_DAT_NAME)).unwrap(),
        VALUES_DAT
    );
    assert_eq!(
        fs::read(dir.path().join(NETWORK_ZST)).unwrap(),
        compress(NETWORK_JSON.as_bytes())
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(STAMP_FILE))
            .unwrap()
            .trim(),
        pin.sha256
    );

    // The stamp makes a second call a no-op: a zip that no longer exists is never opened
    fs::remove_file(&zip_path).unwrap();
    unpack_model(&zip_path, &pin, dir.path()).unwrap();
}

#[test_case(Tamper::Archive; "wrong archive sha")]
#[test_case(Tamper::Member; "wrong member sha")]
#[test_case(Tamper::Decompressed; "wrong decompressed sha")]
fn unpack_verifies_pin(tamper: Tamper) {
    let Archive {
        zip_path,
        mut pin,
        _scratch,
    } = archive();
    let dir = TempDir::new().unwrap();
    match tamper {
        Tamper::Archive => pin.sha256 = sha256_hex(b"other"),
        Tamper::Member => {
            pin.file_sha256
                .insert(NETWORK_ZST.to_owned(), sha256_hex(b"other"));
        }
        Tamper::Decompressed => {
            pin.decompressed_sha256
                .insert(VALUES_DAT_NAME.to_owned(), sha256_hex(b"other"));
        }
    }

    let err = unpack_model(&zip_path, &pin, dir.path()).unwrap_err();

    assert!(is_sha_mismatch(&err), "{err:?}");
    assert!(!dir.path().join(STAMP_FILE).exists());
    assert!(!dir.path().join(VALUES_DAT_NAME).exists());
    assert!(!dir.path().join(NETWORK_ZST).exists());
}

#[derive(Clone, Copy, Debug)]
enum Tamper {
    Archive,
    Member,
    Decompressed,
}

#[test]
fn unpacked_model_opens_and_looks_up_values() {
    let Archive {
        zip_path,
        pin,
        _scratch,
    } = archive();
    let dir = TempDir::new().unwrap();
    unpack_model(&zip_path, &pin, dir.path()).unwrap();

    let model = Model::open(dir.path()).unwrap();

    assert_eq!(model.lookup_text("0").unwrap(), FIRST_VALUE);
    assert_eq!(model.lookup_text(SECOND_ID).unwrap(), SECOND_VALUE);
    assert!(model.lookup_text(OUT_OF_RANGE_ID).is_err());
    let names: Vec<&str> = model
        .network()
        .nodes
        .iter()
        .map(|node| node.name.as_str())
        .collect();
    assert_eq!(names, ["zeta", "alpha"]);
}

#[test]
fn ordered_keeps_document_order() {
    let parsed = parse_ordered(r#"{"b":1,"a":[true,null,"x",1.5],"10":{"2":0,"1":0}}"#).unwrap();

    let Ordered::Object(entries) = parsed else {
        panic!("not an object")
    };
    let keys: Vec<&str> = entries.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(keys, ["b", "a", "10"]);
    assert_eq!(entries[0].1, Ordered::Number(Number::from(1)));
    let Ordered::Object(inner) = &entries[2].1 else {
        panic!("not an object")
    };
    assert_eq!(inner[0].0, "2");
}

#[allow(clippy::disallowed_macros)]
#[test]
fn open_real_model() {
    let Some(dir) = std::env::var_os(MODEL_DIR_ENV) else {
        eprintln!("skipped: {MODEL_DIR_ENV} is not set");
        return;
    };

    let model = Model::open(Path::new(&dir)).unwrap();

    let network = model.network();
    let browser = &network.nodes[0];
    assert_eq!(browser.name, "browser");
    let first_id = &network.ids[browser.possible[0] as usize];
    let text = model.lookup_text(first_id).unwrap();
    assert!(parse_ordered(&text).is_ok(), "{text}");
}

// Value ids are base64.b85encode(bytes([position])): 00 0R 0s 0{ 1O 1p 1^
const TINY_VALUES: [&str; 7] = [
    r#""Linux""#,
    r#""Windows""#,
    r#""macOS""#,
    r#""Firefox""#,
    r#""Chrome""#,
    r#"{"width":1920,"height":1080}"#,
    r#"{"width":1366,"height":768}"#,
];
// os -> browser -> screen
const TINY_NETWORK: &str = r#"{"nodes":[
    {"name":"os","parentNames":[],"possibleValues":["00","0R","0s"],
     "conditionalProbabilities":{"00":0.5,"0R":0.3,"0s":0.2}},
    {"name":"browser","parentNames":["os"],"possibleValues":["0{","1O"],
     "conditionalProbabilities":{"00":{"0{":0.7,"1O":0.3},"0R":{"0{":0.4,"1O":0.6},"0s":{"0{":0.5,"1O":0.5}}},
    {"name":"screen","parentNames":["browser"],"possibleValues":["1p","1^"],
     "conditionalProbabilities":{"0{":{"1p":0.6,"1^":0.4},"1O":{"1p":0.1,"1^":0.9}}}
]}"#;
const SAMPLES: usize = 10_000;
const TOLERANCE: f64 = 0.02;

fn tiny_model() -> (Arc<Model>, TempDir) {
    let dir = TempDir::new().unwrap();
    let mut index = String::from("{");
    let mut dat = String::new();
    for (position, value) in TINY_VALUES.iter().enumerate() {
        if position > 0 {
            index.push(',');
        }
        index.push_str(&format!("\"{:x}\":{}", dat.len(), value.len()));
        dat.push_str(value);
    }
    index.push('}');

    fs::write(
        dir.path().join(NETWORK_ZST),
        compress(TINY_NETWORK.as_bytes()),
    )
    .unwrap();
    fs::write(dir.path().join(VALUES_JSON_ZST), compress(index.as_bytes())).unwrap();
    fs::write(dir.path().join(VALUES_DAT_NAME), dat).unwrap();
    (Model::open(dir.path()).unwrap(), dir)
}

fn conditions(entries: [(&str, Constraint); 1]) -> Conditions {
    Conditions(
        entries
            .into_iter()
            .map(|(key, c)| (key.to_owned(), c))
            .collect(),
    )
}

fn draws(model: &Model, conditions: &Conditions, count: usize) -> Vec<Value> {
    let mut rng = PyRandom::seed(1);
    (0..count)
        .map(|_| generate(model, conditions, &mut rng).unwrap())
        .collect()
}

fn share(fingerprints: &[Value], matches: impl Fn(&Value) -> bool) -> f64 {
    fingerprints.iter().filter(|fp| matches(fp)).count() as f64 / fingerprints.len() as f64
}

fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < TOLERANCE,
        "{what}: {actual} vs {expected}"
    );
}

#[test]
fn sampling_matches_tables() {
    let (model, _dir) = tiny_model();

    let fingerprints = draws(&model, &Conditions(Vec::new()), SAMPLES);

    for (os, expected) in [("Linux", 0.5), ("Windows", 0.3), ("macOS", 0.2)] {
        assert_close(share(&fingerprints, |fp| fp["os"] == os), expected, os);
    }
    // joint of os and browser: P(os) * P(browser | os)
    for (os, browser, expected) in [
        ("Linux", "Firefox", 0.35),
        ("Linux", "Chrome", 0.15),
        ("Windows", "Firefox", 0.12),
        ("Windows", "Chrome", 0.18),
        ("macOS", "Firefox", 0.10),
        ("macOS", "Chrome", 0.10),
    ] {
        let actual = share(&fingerprints, |fp| {
            fp["os"] == os && fp["browser"] == browser
        });
        assert_close(actual, expected, &format!("{os} {browser}"));
    }
    // P(browser) * P(width | browser)
    for (browser, width, expected) in [
        ("Firefox", 1920, 0.57 * 0.6),
        ("Firefox", 1366, 0.57 * 0.4),
        ("Chrome", 1920, 0.43 * 0.1),
        ("Chrome", 1366, 0.43 * 0.9),
    ] {
        let actual = share(&fingerprints, |fp| {
            fp["browser"] == browser && fp["screen"]["width"] == width
        });
        assert_close(actual, expected, &format!("{browser} {width}"));
    }
}

#[test]
fn sampling_any_of_excludes_the_rest() {
    let (model, _dir) = tiny_model();
    let any_of = conditions([(
        "OS",
        Constraint::AnyOf(vec![json!("linux"), json!("WINDOWS")]),
    )]);

    let fingerprints = draws(&model, &any_of, SAMPLES);

    assert!(fingerprints.iter().all(|fp| fp["os"] != "macOS"));
    // Renormalized over the two allowed values
    assert_close(
        share(&fingerprints, |fp| fp["os"] == "Linux"),
        0.5 / 0.8,
        "Linux",
    );
}

#[test]
fn sampling_dotted_path_filters_before_drawing() {
    let (model, _dir) = tiny_model();
    let narrow = conditions([("screen.width", Constraint::Equals(json!(1366)))]);

    let fingerprints = draws(&model, &narrow, 1000);

    assert!(fingerprints.iter().all(|fp| fp["screen"]["width"] == 1366));
}

#[test]
fn sampling_later_condition_on_a_node_replaces_the_earlier() {
    let (model, _dir) = tiny_model();
    let replaced = Conditions(vec![
        ("os".to_owned(), Constraint::Equals(json!("Linux"))),
        ("OS".to_owned(), Constraint::Equals(json!("Windows"))),
    ]);

    let fingerprints = draws(&model, &replaced, 100);

    assert!(fingerprints.iter().all(|fp| fp["os"] == "Windows"));
}

#[test_case("screen.width", json!(9999); "a path value no node holds")]
#[test_case("browser", json!("Netscape"); "a node value no node holds")]
#[test_case("nothing.width", json!(1); "an unknown node")]
fn sampling_unsatisfiable(key: &str, value: Value) {
    let (model, _dir) = tiny_model();
    let unsatisfiable = conditions([(key, Constraint::Equals(value))]);

    let err = generate(&model, &unsatisfiable, &mut PyRandom::seed(1)).unwrap_err();

    assert!(
        matches!(&err, Error::InvalidArgument { field, .. } if field == "fingerprint"),
        "{err:?}"
    );
}

#[allow(clippy::disallowed_macros)]
#[test]
fn real_model_constraints() {
    let Some(dir) = std::env::var_os(MODEL_DIR_ENV) else {
        eprintln!("skipped: {MODEL_DIR_ENV} is not set");
        return;
    };
    let model = Model::open(Path::new(&dir)).unwrap();
    let firefox_on_linux = Conditions(vec![
        ("browser".to_owned(), Constraint::Equals(json!("Firefox"))),
        ("os".to_owned(), Constraint::Equals(json!("Linux"))),
    ]);

    let first = generate(&model, &firefox_on_linux, &mut PyRandom::seed(7)).unwrap();
    let second = generate(&model, &firefox_on_linux, &mut PyRandom::seed(7)).unwrap();

    assert_eq!(first, second);
    let user_agent = first["navigator"]["userAgent"].as_str().unwrap();
    assert!(
        user_agent.contains("Firefox") && user_agent.contains("Linux"),
        "{user_agent}"
    );
    assert!(first["screen"]["width"].as_u64().unwrap() > 0);

    // The rare case the model can answer only by filtering before the draw
    let wide = Conditions(vec![
        ("os".to_owned(), Constraint::Equals(json!("Linux"))),
        ("screen.width".to_owned(), Constraint::Equals(json!(2560))),
    ]);
    let fingerprint = generate(&model, &wide, &mut PyRandom::seed(7)).unwrap();
    assert_eq!(fingerprint["screen"]["width"], 2560);
}
