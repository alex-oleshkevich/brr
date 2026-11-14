use std::{path::Path, sync::Arc};

use brr_core::{Error, Size};
use serde_json::{Value, json};

use super::{IdentityInput, TargetOs, crc32, float_sum, identity, py_sum};
use crate::{fpgen::model::Model, pyrandom::PyRandom};

const MODEL_DIR_ENV: &str = "BRR_TEST_FPGEN_MODEL";

fn read(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/identity")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn load(name: &str) -> Value {
    serde_json::from_str(&read(name)).unwrap()
}

#[test]
fn crc32_matches_zlib() {
    for case in load("pycompat.json")["crc32"].as_array().unwrap() {
        let data = case[0].as_str().unwrap().as_bytes();
        assert_eq!(json!(crc32(data)), case[1], "{case}");
    }
}

#[test]
fn python_sums() {
    let recorded = load("pycompat.json");

    for case in recorded["sums"].as_array().unwrap() {
        let values: Vec<Value> = case["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item.get("i").or_else(|| item.get("f")).unwrap().clone())
            .collect();
        let expected = case["result"].get("i").or_else(|| case["result"].get("f"));
        assert_eq!(
            py_sum(&values),
            expected.unwrap().as_f64().unwrap(),
            "{case}"
        );
    }
    for case in recorded["floatSums"].as_array().unwrap() {
        let values: Vec<f64> = case["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_f64().unwrap())
            .collect();
        assert_eq!(float_sum(&values), case["sum"].as_f64().unwrap(), "{case}");
    }
}

fn model() -> Option<Arc<Model>> {
    let Some(dir) = std::env::var_os(MODEL_DIR_ENV) else {
        #[allow(clippy::disallowed_macros)]
        {
            eprintln!("skipped: {MODEL_DIR_ENV} is not set");
        }
        return None;
    };
    Some(Model::open(Path::new(&dir)).unwrap())
}

fn linux_input() -> IdentityInput {
    IdentityInput {
        os: Some(TargetOs::Linux),
        screen: Some(Size {
            width: 1920,
            height: 1080,
        }),
        fonts: Vec::new(),
        webgl: None,
        locale: None,
        ff_version: 158,
    }
}

#[test]
fn identity_real_model() {
    let Some(model) = model() else {
        return;
    };

    let first = identity(&model, &linux_input(), &mut PyRandom::seed(3)).unwrap();
    let second = identity(&model, &linux_input(), &mut PyRandom::seed(3)).unwrap();

    assert_eq!(first, second);
    let user_agent = first["navigator.userAgent"].as_str().unwrap();
    assert!(user_agent.contains("Firefox/158.0"), "{user_agent}");
    assert!(user_agent.contains("Linux"), "{user_agent}");
    assert!(first["screen.width"].as_i64().unwrap() <= 1920);
    assert!(!first["fonts"].as_array().unwrap().is_empty());
    assert!(first["webGl:renderer"].is_string());
    assert!(!first["voices"].as_array().unwrap().is_empty());

    let with_fonts = IdentityInput {
        fonts: vec!["Brr Test Sans".to_owned()],
        ..linux_input()
    };
    let drawn = identity(&model, &with_fonts, &mut PyRandom::seed(3)).unwrap();
    assert_eq!(drawn["fonts"], json!(["Brr Test Sans"]));

    let fake_gpu = IdentityInput {
        os: Some(TargetOs::Windows),
        webgl: Some(("Fake".to_owned(), "Fake".to_owned())),
        ..linux_input()
    };
    let err = identity(&model, &fake_gpu, &mut PyRandom::seed(3)).unwrap_err();
    assert!(
        matches!(&err, Error::InvalidArgument { field, .. } if field == "fingerprint.webgl"),
        "{err:?}"
    );

    let long_name = IdentityInput {
        fonts: vec!["x".repeat(super::MAX_FONT_NAME_CHARS + 1)],
        ..linux_input()
    };
    let err = identity(&model, &long_name, &mut PyRandom::seed(3)).unwrap_err();
    assert!(
        matches!(&err, Error::InvalidArgument { field, .. } if field == "fingerprint.fonts"),
        "{err:?}"
    );

    let long_gpu = IdentityInput {
        webgl: Some(("v".repeat(10_000), "r".repeat(10_000))),
        ..fake_gpu
    };
    let Err(Error::InvalidArgument { message, .. }) =
        identity(&model, &long_gpu, &mut PyRandom::seed(3))
    else {
        panic!("a fake GPU is refused");
    };
    assert!(message.len() < 1_000, "{}", message.len());

    let too_many = IdentityInput {
        fonts: vec!["x".to_owned(); super::MAX_EXTRA_FONTS + 1],
        ..linux_input()
    };
    let err = identity(&model, &too_many, &mut PyRandom::seed(3)).unwrap_err();
    assert!(
        matches!(&err, Error::InvalidArgument { field, .. } if field == "fingerprint.fonts"),
        "{err:?}"
    );
}

#[test]
fn identity_names_a_recorded_gpu() {
    let Some(model) = model() else {
        return;
    };
    let (vendor, renderer) = crate::webgl::firefox_gpus(&model, "lin").unwrap().remove(0);
    let input = IdentityInput {
        webgl: Some((vendor.clone(), renderer.clone())),
        ..linux_input()
    };

    let config = identity(&model, &input, &mut PyRandom::seed(5)).unwrap();

    assert_eq!(config["webGl:vendor"], json!(vendor));
    assert_eq!(config["webGl:renderer"], json!(renderer));
}
