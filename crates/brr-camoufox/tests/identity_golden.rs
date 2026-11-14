#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use brr_camoufox::{
    coherence::{self, Config},
    data::{
        ESSENTIAL_FONTS, FINGERPRINT_PRESETS, FINGERPRINT_PRESETS_V150, FPGEN, LAUNCHER_CONSTANTS,
        VOICE_MANIFESTS_JSON,
    },
    fingerprints::{
        self, fonts, init_script, media, preset,
        screen::{self, HostCpu, Screen},
        voices,
    },
    fpgen::model::{Model, Ordered, parse_ordered},
    locales, pycompat,
    pyrandom::PyRandom,
    webgl,
};
use brr_core::Error;
use serde_json::{Number, Value, json};

const MODEL_DIR_ENV: &str = "BRR_TEST_FPGEN_MODEL";
const OS_KEYS: [&str; 3] = ["win", "mac", "lin"];
const REPLAYED: [&str; 13] = [
    "coherence.json",
    "constants.json",
    "fonts.json",
    "fpgen-salts.json",
    "from-fpgen.json",
    "geometry.json",
    "init-script.json",
    "media.json",
    "numpy.json",
    "presets.json",
    "pycompat.json",
    "voices.json",
    "webgl.json",
];
const DETAILED_CASES: usize = 40;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/identity")
}

fn load(name: &str) -> Value {
    let text = fs::read_to_string(fixtures().join(name)).unwrap();
    serde_json::from_str(&text).unwrap()
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

fn text(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn config(value: &Value) -> Config {
    value.as_object().cloned().unwrap()
}

fn violations(list: &[coherence::Violation]) -> Value {
    list.iter()
        .map(|violation| json!([violation.rule, violation.detail]))
        .collect()
}

fn rule_names(list: &[coherence::Violation]) -> Value {
    list.iter().map(|violation| json!(violation.rule)).collect()
}

#[test]
fn every_fixture_is_replayed() {
    let on_disk: BTreeSet<String> = fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    let known: BTreeSet<String> = REPLAYED.iter().map(|name| (*name).to_owned()).collect();

    let unknown: Vec<&String> = on_disk.difference(&known).collect();

    assert!(unknown.is_empty(), "fixtures nobody replays: {unknown:?}");
}

#[test]
fn numpy_pairwise_sum() {
    for case in load("numpy.json")["sums"].as_array().unwrap() {
        let values: Vec<f64> = case["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_f64().unwrap())
            .collect();

        assert_eq!(
            locales::pairwise_sum(&values),
            case["sum"].as_f64().unwrap(),
            "n={}",
            values.len()
        );
    }
}

#[test]
fn coherence_fits_os() {
    for case in load("coherence.json")["fits"].as_array().unwrap() {
        let actual = coherence::gpu_fits_os(case[0].as_str(), text(&case[1]));
        assert_eq!(json!(actual), case[2], "{case}");
    }
}

#[test]
fn coherence_fits_machine() {
    for case in load("coherence.json")["machine"].as_array().unwrap() {
        let actual = coherence::gpu_fits_machine(
            case[0].as_str(),
            text(&case[1]),
            &case[2],
            &case[3],
            &case[4],
        );
        assert_eq!(json!(actual), case[5], "{case}");
    }
}

#[test]
fn gpu_screen_plausibility() {
    for case in load("coherence.json")["gpuScreen"].as_array().unwrap() {
        let renderer = case[0].as_str();
        let (width, height) = (case[1].as_i64(), case[2].as_i64());

        assert_eq!(
            json!(webgl::gpu_screen_is_plausible(renderer, width, height)),
            case[3],
            "{case}"
        );
        assert_eq!(
            json!(webgl::is_software_renderer(renderer)),
            case[4],
            "{case}"
        );
        assert_eq!(
            renderer
                .map(webgl::renderer_bucket)
                .map(|b| json!(b))
                .unwrap_or(Value::Null),
            case[5],
            "{case}"
        );
    }
}

#[test]
fn coherence_cases() {
    for (index, case) in load("coherence.json")["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let target = text(&case["os"]);
        let input = config(&case["input"]);

        let found = coherence::validate(&input, target);
        assert_eq!(rule_names(&found), case["rules"], "case {index}");
        assert_eq!(
            json!(pycompat::canon_digest(&violations(&found))),
            case["validate"],
            "case {index}"
        );

        let mut applied = input.clone();
        let left = coherence::apply(&mut applied, target);
        assert_eq!(rule_names(&left), case["apply"], "case {index}");
        assert_eq!(
            json!(pycompat::canon_digest(&violations(&left))),
            case["applyHash"],
            "case {index}"
        );
        assert_eq!(
            json!(pycompat::canon_digest(&Value::Object(applied.clone()))),
            case["applied"],
            "case {index}"
        );

        let mut dropped_config = input.clone();
        let dropped = coherence::drop_incoherent_source_values(&mut dropped_config, target);
        assert_eq!(violations(&dropped), case["dropped"], "case {index}");
        assert_eq!(
            json!(pycompat::canon_digest(&Value::Object(dropped_config))),
            case["droppedConfig"],
            "case {index}"
        );

        assert_eq!(
            json!(coherence::screen_is_implausible(&input)),
            case["implausible"],
            "case {index}"
        );

        if index < DETAILED_CASES {
            assert_eq!(
                pycompat::canon(&violations(&found)),
                pycompat::canon(&case["validateFull"]),
                "case {index}"
            );
            assert_eq!(
                pycompat::canon(&Value::Object(applied)),
                pycompat::canon(&case["appliedFull"]),
                "case {index}"
            );
        }
    }
}

fn message(result: Result<Ordered, Error>) -> Result<String, String> {
    match result {
        Ok(config) => Ok(pycompat::digest(&config)),
        Err(Error::InvalidArgument { message, .. }) => Err(message),
        Err(other) => panic!("unexpected error {other}"),
    }
}

fn seed(case: &Value) -> Option<u128> {
    Some(match &case["seed"] {
        Value::String(digits) => digits.parse().unwrap(),
        number => u128::from(number.as_u64().unwrap()),
    })
}

fn renderer_of(config: &Ordered) -> &str {
    let Ordered::Object(members) = config else {
        panic!("a config is an object");
    };
    match &members
        .iter()
        .find(|(key, _)| key == "webGl:renderer")
        .unwrap()
        .1
    {
        Ordered::String(renderer) => renderer,
        other => panic!("renderer is {other:?}"),
    }
}

#[test]
fn webgl_gpus() {
    let Some(model) = model() else { return };

    for (os_key, recorded) in load("webgl.json")["gpus"].as_object().unwrap() {
        let gpus: Vec<Value> = webgl::firefox_gpus(&model, os_key)
            .unwrap()
            .into_iter()
            .map(|(vendor, renderer)| json!([vendor, renderer]))
            .collect();
        assert_eq!(&Value::Array(gpus), recorded, "{os_key}");
    }
    assert_eq!(
        load("webgl.json")["gpus"]
            .as_object()
            .unwrap()
            .keys()
            .count(),
        OS_KEYS.len()
    );
}

#[test]
fn webgl_for_screen_and_machine() {
    let Some(model) = model() else { return };
    let golden = load("webgl.json");

    for group in ["forScreen", "forMachine"] {
        for case in golden[group].as_array().unwrap() {
            let config = webgl::sample_for_screen(
                &model,
                text(&case["os"]),
                case["w"].as_i64(),
                case["h"].as_i64(),
                seed(case),
                case["cores"].as_i64(),
            )
            .unwrap();

            assert_eq!(json!(renderer_of(&config)), case["renderer"], "{case}");
            assert_eq!(json!(pycompat::digest(&config)), case["hash"], "{case}");
        }
    }
}

#[test]
fn webgl_for_gpu() {
    let Some(model) = model() else { return };

    for case in load("webgl.json")["forGpu"].as_array().unwrap() {
        let result = message(webgl::for_gpu(
            &model,
            text(&case["os"]),
            text(&case["vendor"]),
            text(&case["renderer"]),
            seed(case),
        ));

        match (result, case.get("ok")) {
            (Ok(hash), Some(expected)) => assert_eq!(json!(hash), *expected, "{case}"),
            (Err(actual), None) => assert_eq!(json!(actual), case["message"], "{case}"),
            (other, _) => panic!("{case}: {other:?}"),
        }
    }
}

#[test]
fn webgl_unknown_os() {
    let Some(model) = model() else { return };
    let expected = &load("webgl.json")["errors"]["unknownOs"];

    let result = message(webgl::sample_for_screen(
        &model,
        "bsd",
        Some(1920),
        Some(1080),
        Some(0),
        None,
    ));

    assert_eq!(result.unwrap_err(), text(&expected["message"]));
}

fn member<'a>(value: &'a Ordered, key: &str) -> &'a Ordered {
    let Ordered::Object(members) = value else {
        panic!("{key}: not an object");
    };
    &members.iter().find(|(name, _)| name == key).unwrap().1
}

#[test]
fn webgl_to_config() {
    let root = parse_ordered(&fs::read_to_string(fixtures().join("webgl.json")).unwrap()).unwrap();
    let recorded = member(&root, "recorded");
    let Ordered::Array(converted) = member(&root, "converted") else {
        panic!("converted is a list");
    };

    for case in converted {
        let (Ordered::String(os), Ordered::String(hash)) =
            (member(case, "os"), member(case, "hash"))
        else {
            panic!("converted case without os or hash");
        };

        let config = webgl::to_config(recorded, member(case, "webgl2"), os).unwrap();

        assert_eq!(&pycompat::digest(&config), hash, "{os}");
    }
}

#[test]
fn pycompat_floats_and_reprs() {
    let golden = load("pycompat.json");

    for case in golden["floats"].as_array().unwrap() {
        let float = case[0].as_f64().unwrap();
        let orjson = pycompat::orjson_dumps(&Ordered::Number(Number::from_f64(float).unwrap()));
        assert_eq!(json!(orjson), case[1], "{case}");
        assert_eq!(json!(pycompat::float_repr(float)), case[2], "{case}");
    }
    for case in golden["reprs"].as_array().unwrap() {
        assert_eq!(json!(pycompat::str_repr(text(&case[0]))), case[1], "{case}");
    }
}

const SALT: u64 = 12_345_678_901_234_567_890;

fn seed_of(value: &Value) -> u128 {
    match value {
        Value::String(digits) => digits.parse().unwrap(),
        number => u128::from(number.as_u64().unwrap()),
    }
}

fn rng_of(value: &Value) -> PyRandom {
    PyRandom::seed_wide(seed_of(value))
}

fn digest(value: &Value) -> String {
    pycompat::canon_digest(value)
}

fn config_digest(config: &Config) -> String {
    digest(&Value::Object(config.clone()))
}

fn assert_same(actual: &Value, expected: &Value, context: &Value) {
    assert_eq!(
        pycompat::canon(actual),
        pycompat::canon(expected),
        "{context}"
    );
}

fn sorted_keys(config: &Config) -> Vec<&String> {
    let mut keys: Vec<&String> = config.keys().collect();
    keys.sort();
    keys
}

fn ff_version(value: &Value) -> Option<&str> {
    value.as_str()
}

#[test]
fn fingerprints_constants() {
    let constants = load("constants.json");
    let sorted = |value: &Value| {
        let mut list: Vec<String> = value
            .as_array()
            .unwrap()
            .iter()
            .map(|item| text(item).to_owned())
            .collect();
        list.sort();
        list
    };

    assert_eq!(constants["fpgenData"], *FPGEN);
    for (golden, key) in [
        ("essentialMacos", "mac"),
        ("essentialWindows", "win"),
        ("essentialLinux", "lin"),
    ] {
        assert_eq!(constants[golden], ESSENTIAL_FONTS[key], "{golden}");
    }
    for (golden, key) in [("macos", "mac"), ("windows", "win"), ("linux", "lin")] {
        assert_eq!(
            constants["markers"][golden], LAUNCHER_CONSTANTS["markerFonts"][key],
            "{golden}"
        );
    }
    assert_eq!(
        constants["windows11Markers"],
        json!(sorted(&LAUNCHER_CONSTANTS["windows11MarkerFonts"]))
    );
    assert_eq!(
        constants["plausibleCoreCounts"],
        LAUNCHER_CONSTANTS["plausibleCoreCounts"]
    );
    assert_eq!(
        constants["modernScreenFloor"],
        json!([
            screen::MODERN_SCREEN_FLOOR.0 as i64,
            screen::MODERN_SCREEN_FLOOR.1 as i64
        ])
    );
    assert_eq!(
        constants["macNovelty"],
        json!(sorted(&LAUNCHER_CONSTANTS["macNoveltyVoices"]))
    );
    assert_eq!(
        constants["macEloquence"],
        json!(sorted(&LAUNCHER_CONSTANTS["macEloquenceVoices"]))
    );
    assert_eq!(
        constants["presetsV150MinFf"],
        json!(preset::PRESETS_V150_MIN_FF)
    );
}

#[test]
fn fingerprints_salts_and_seeds() {
    for case in load("pycompat.json")["salts"].as_array().unwrap() {
        let pinned = match case.get("screen") {
            Some(bounds) => json!({
                "min_width": bounds[0], "max_width": bounds[1],
                "min_height": bounds[2], "max_height": bounds[3],
            }),
            None => case["value"].clone(),
        };
        // identity_salt(None) is a fresh random salt: nothing to compare
        if pinned.is_null() {
            continue;
        }
        assert_eq!(
            json!(fingerprints::identity_salt(&pinned).to_string()),
            case["salt"],
            "{case}"
        );
    }
    for case in load("pycompat.json")["seeds"].as_array().unwrap() {
        let salt: u64 = text(&case["salt"]).parse().unwrap();
        assert_eq!(
            json!(fingerprints::identity_seed(&config(&case["config"]), salt)),
            case["seed"],
            "{case}"
        );
    }
    for case in load("fpgen-salts.json").as_array().unwrap() {
        assert_eq!(
            json!(fingerprints::identity_salt(&case["fingerprint"]).to_string()),
            case["salt"],
            "{}",
            case["label"]
        );
    }
}

#[test]
fn fingerprints_fonts() {
    for case in load("fonts.json")["cases"].as_array().unwrap() {
        let drawn = fonts::generate_random_font_subset(
            text(&case["os"]),
            &mut rng_of(&case["seed"]),
            case["native"].as_bool().unwrap_or(false),
            case["locale"].as_str(),
        );
        let list = json!(drawn);

        assert_eq!(json!(drawn.len()), case["len"], "{case}");
        assert_eq!(json!(digest(&list)), case["hash"], "{case}");
        if let Some(recorded) = case.get("fonts") {
            assert_eq!(&list, recorded, "{case}");
        }
    }
}

// Every "Name:lang:type" entry in a voice manifest, in document order
fn manifest_entries(node: &Ordered, out: &mut Vec<String>) {
    match node {
        Ordered::Object(members) => {
            for (_, member) in members {
                manifest_entries(member, out);
            }
        }
        Ordered::Array(list) => {
            for item in list {
                match item {
                    Ordered::String(entry) if entry.matches(':').count() >= 2 => {
                        out.push(entry.clone());
                    }
                    other => manifest_entries(other, out),
                }
            }
        }
        _ => {}
    }
}

fn presets_of(file: &str) -> &'static Value {
    match file {
        "fingerprint-presets.json" => &FINGERPRINT_PRESETS,
        _ => &FINGERPRINT_PRESETS_V150,
    }
}

#[test]
fn fingerprints_voices() {
    let recorded = load("voices.json");

    for case in recorded["cases"].as_array().unwrap() {
        let drawn = voices::generate_random_voice_subset(
            text(&case["os"]),
            case["locale"].as_str(),
            &mut rng_of(&case["seed"]),
        )
        .unwrap();
        let list = Value::Array(drawn);

        assert_eq!(json!(list.as_array().unwrap().len()), case["len"], "{case}");
        assert_eq!(json!(digest(&list)), case["hash"], "{case}");
        if let Some(voices) = case.get("voices") {
            assert_same(&list, voices, case);
        }
    }

    for case in recorded["uris"].as_array().unwrap() {
        let uri = voices::voice_uri(text(&case[0]), text(&case[1]), text(&case[2]));
        assert_eq!(json!(uri), case[3], "{case}");
    }

    let Ordered::Object(manifests) = parse_ordered(VOICE_MANIFESTS_JSON).unwrap() else {
        panic!("manifests are an object");
    };
    for (os_key, manifest) in &manifests {
        let mut entries = Vec::new();
        manifest_entries(manifest, &mut entries);
        let uris: Vec<String> = entries
            .iter()
            .map(|entry| {
                let (rest, _) = entry.rsplit_once(':').unwrap();
                let (name, lang) = rest.rsplit_once(':').unwrap();
                voices::voice_uri(os_key, name, lang)
            })
            .collect();
        assert_eq!(
            json!(digest(&json!(uris))),
            recorded["uriHashes"][os_key],
            "{os_key}"
        );
    }

    for case in recorded["normalized"].as_array().unwrap() {
        if let Some(extra) = case.get("extra") {
            let out = voices::normalize_preset_voices(extra.as_array().unwrap(), text(&case["os"]));
            assert_eq!(Value::Array(out), case["out"], "{case}");
            continue;
        }
        let entry = &presets_of(text(&case["file"]))["presets"][text(&case["os"])]
            [case["index"].as_u64().unwrap() as usize];
        let out = Value::Array(voices::normalize_preset_voices(
            entry["speechVoices"].as_array().unwrap(),
            text(&case["os"]),
        ));
        assert_eq!(json!(out.as_array().unwrap().len()), case["len"], "{case}");
        assert_eq!(json!(digest(&out)), case["hash"], "{case}");
    }
}

#[test]
fn fingerprints_media() {
    let recorded = load("media.json");

    for case in recorded["cases"].as_array().unwrap() {
        let out = Value::Object(
            media::draw_media_devices(text(&case["os"]), &mut rng_of(&case["seed"]))
                .into_iter()
                .collect(),
        );

        assert_eq!(json!(digest(&out)), case["hash"], "{case}");
        if let Some(recorded) = case.get("out") {
            assert_same(&out, recorded, case);
        }
    }

    for case in recorded["defaults"].as_array().unwrap() {
        let mut config = config(&case["config"]);
        media::set_media_devices_defaults(&mut config, text(&case["salt"]).parse().unwrap());
        assert_eq!(json!(config_digest(&config)), case["hash"], "{case}");
    }
}

fn run_geometry(
    name: &str,
    config: &mut Config,
    os: &str,
    caps: (Option<i64>, Option<i64>),
) -> Option<bool> {
    match name {
        "followBaselineProtection" => screen::follow_baseline_protection(config, os),
        "clampWindowDimensions" => screen::clamp_window_dimensions(config),
        "clampScreenToDisplay" => screen::clamp_screen_to_display(config, caps.0, caps.1),
        "clampWindowPosition" => screen::clamp_window_position(config),
        "raiseScreenToModernFloor" => screen::raise_screen_to_modern_floor(config),
        "repairScreenOrientation" => return Some(coherence::repair_screen_orientation(config)),
        other => panic!("unknown geometry fix {other}"),
    }
    None
}

#[test]
fn fingerprints_geometry() {
    let recorded = load("geometry.json");

    for case in recorded["cases"].as_array().unwrap() {
        let os = text(&case["os"]);
        let caps = (case["capW"].as_i64(), case["capH"].as_i64());

        for (name, expected) in case["out"].as_object().unwrap() {
            let mut config = config(&case["input"]);
            let ret = if name == "pipeline" {
                if coherence::screen_is_implausible(&config) {
                    coherence::repair_screen_orientation(&mut config);
                    screen::raise_screen_to_modern_floor(&mut config);
                }
                screen::raise_screen_to_modern_floor(&mut config);
                screen::clamp_screen_to_display(&mut config, caps.0, caps.1);
                screen::follow_baseline_protection(&mut config, os);
                screen::clamp_window_dimensions(&mut config);
                screen::clamp_window_position(&mut config);
                None
            } else {
                run_geometry(name, &mut config, os, caps)
            };

            let context = json!({"fix": name, "os": os, "input": case["input"]});
            // The recorded "keys" hashes cover Python's key order, which the config does not keep
            match expected.get("config") {
                Some(full) => assert_same(&Value::Object(config.clone()), full, &context),
                None => assert_eq!(json!(config_digest(&config)), expected["hash"], "{context}"),
            }
            if let Some(ret) = ret {
                assert_eq!(json!(ret), expected["ret"], "{context}");
            }
        }
    }

    for case in recorded["arch"].as_array().unwrap() {
        let mut config = Config::new();
        config.insert("navigator.userAgent".to_owned(), case[0].clone());
        for (index, key) in [(1, "navigator.platform"), (2, "navigator.oscpu")] {
            if !case[index].is_null() {
                config.insert(key.to_owned(), case[index].clone());
            }
        }
        screen::fix_navigator_arch(&mut config, text(&case[3]));
        assert_eq!(
            config.get("navigator.platform").unwrap_or(&Value::Null),
            &case[4],
            "{case}"
        );
        assert_eq!(
            config.get("navigator.oscpu").unwrap_or(&Value::Null),
            &case[5],
            "{case}"
        );
    }

    for case in recorded["hardwareConcurrency"].as_array().unwrap() {
        let mut config = Config::new();
        if !case[0].is_null() {
            config.insert("navigator.hardwareConcurrency".to_owned(), case[0].clone());
        }
        let host = HostCpu {
            cpu_count: case[1].as_u64().map(|count| count as u32),
            can_pin_host: case[2].as_bool().unwrap(),
        };
        screen::fix_hardware_concurrency(&mut config, case[3].as_bool(), host);

        match config.get("navigator.hardwareConcurrency") {
            Some(cores) => assert_eq!(cores, &case[4], "{case}"),
            None => assert_eq!(case[4], "__missing__", "{case}"),
        }
    }
}

#[test]
fn fingerprints_from_fpgen() {
    let text_of = fs::read_to_string(fixtures().join("from-fpgen.json")).unwrap();
    let Ordered::Object(document) = parse_ordered(&text_of).unwrap() else {
        panic!("from-fpgen.json is an object");
    };
    let ordered_inputs = match &document.iter().find(|(key, _)| key == "inputs").unwrap().1 {
        Ordered::Array(inputs) => inputs,
        other => panic!("inputs: {other:?}"),
    };
    let recorded = load("from-fpgen.json");

    for case in recorded["cases"].as_array().unwrap() {
        let input = &ordered_inputs[case["input"].as_u64().unwrap() as usize];
        let config = fingerprints::from_fpgen(
            input,
            ff_version(&case["ffVersion"]),
            &mut PyRandom::seed(case["moduleSeed"].as_u64().unwrap()),
        );

        assert_same(&Value::Object(config.clone()), &case["config"], case);
        let mut recorded_keys: Vec<&str> = case["configKeys"]
            .as_array()
            .unwrap()
            .iter()
            .map(text)
            .collect();
        recorded_keys.sort_unstable();
        assert_eq!(sorted_keys(&config), recorded_keys, "{case}");
        assert_eq!(
            json!(fingerprints::identity_seed(&config, SALT)),
            case["identitySeed"],
            "{case}"
        );
    }

    for case in recorded["windowSize"].as_array().unwrap() {
        let mut fingerprint = recorded["inputs"][case["input"].as_u64().unwrap() as usize].clone();
        fingerprints::handle_window_size(
            &mut fingerprint,
            case["w"].as_i64().unwrap(),
            case["h"].as_i64().unwrap(),
        );
        assert_same(&fingerprint, &case["out"], case);
    }

    for case in recorded["screens"].as_array().unwrap() {
        let bounds: Vec<Option<i64>> = case["bounds"]
            .as_array()
            .unwrap()
            .iter()
            .map(Value::as_i64)
            .collect();
        let bound = Screen {
            min_width: bounds[0],
            max_width: bounds[1],
            min_height: bounds[2],
            max_height: bounds[3],
        };
        let probes = [
            json!(800),
            json!(1366),
            json!(1920),
            json!(2560),
            json!(1080.0),
            json!("1920"),
            Value::Null,
        ];
        let answers = |allows: &dyn Fn(&Value) -> Option<bool>| match allows(&probes[0]) {
            None => Value::Null,
            Some(_) => json!(
                probes
                    .iter()
                    .map(|probe| allows(probe).unwrap())
                    .collect::<Vec<_>>()
            ),
        };

        assert_eq!(
            answers(&|probe| bound.allows_width(probe)),
            case["width"],
            "{case}"
        );
        assert_eq!(
            answers(&|probe| bound.allows_height(probe)),
            case["height"],
            "{case}"
        );
    }
}

fn preset_config_view(config: &Config) -> Value {
    let mut view = config.clone();
    for key in ["fonts", "voices"] {
        if let Some(list) = view.get(key).cloned() {
            view.insert(
                key.to_owned(),
                json!({"hash": digest(&list), "len": list.as_array().unwrap().len()}),
            );
        }
    }
    Value::Object(view)
}

#[test]
fn fingerprints_presets() {
    let recorded = load("presets.json");

    for case in recorded["cases"].as_array().unwrap() {
        let entry = &presets_of(text(&case["file"]))["presets"][text(&case["os"])]
            [case["index"].as_u64().unwrap() as usize];
        let salt: u64 = text(&case["salt"]).parse().unwrap();
        let config = preset::from_preset(entry, ff_version(&case["ffVersion"]), salt).unwrap();

        assert_eq!(json!(config_digest(&config)), case["hash"], "{case}");
        let os_key = match text(&case["os"]) {
            "macos" => "mac",
            "windows" => "win",
            _ => "lin",
        };
        let rules: Vec<&str> = coherence::validate(&config, os_key)
            .iter()
            .map(|violation| violation.rule)
            .collect();
        assert_eq!(json!(rules), case["validate"], "{case}");
    }

    for case in recorded["full"].as_array().unwrap() {
        let entry = &presets_of(text(&case["file"]))["presets"][text(&case["os"])]
            [case["index"].as_u64().unwrap() as usize];
        let claimed = (text(&case["file"]) == "fingerprint-presets-v150.json").then_some("152");
        let config = preset::from_preset(entry, claimed, 0).unwrap();
        assert_same(&preset_config_view(&config), &case["config"], case);
    }

    for case in recorded["synthetic"].as_array().unwrap() {
        let config =
            preset::from_preset(&case["preset"], ff_version(&case["ffVersion"]), 99).unwrap();
        assert_same(&preset_config_view(&config), &case["config"], case);
    }

    for case in recorded["random"].as_array().unwrap() {
        let os: Vec<&str> = match &case["os"] {
            Value::Null => Vec::new(),
            Value::Array(names) => names.iter().map(text).collect(),
            name => vec![text(name)],
        };
        let drawn = preset::get_random_preset(
            &os,
            ff_version(&case["ffVersion"]),
            &mut PyRandom::seed(case["moduleSeed"].as_u64().unwrap()),
        );
        assert_eq!(
            drawn.map_or(Value::Null, |preset| json!(digest(preset))),
            case["hash"],
            "{case}"
        );
    }

    for case in recorded["appVersions"].as_array().unwrap() {
        let derived = preset::app_version_from_user_agent(case[0].as_str());
        assert_eq!(json!(derived), case[1], "{case}");
    }

    for (version, file) in recorded["presetsFile"].as_object().unwrap() {
        let version = (version != "None").then_some(version.as_str());
        assert_eq!(
            json!(preset::select_presets_file(version).name()),
            *file,
            "{version:?}"
        );
    }
}

#[test]
fn fingerprints_init_script() {
    for case in load("init-script.json")["cases"].as_array().unwrap() {
        let script = init_script::build_init_script(case["values"].as_object().unwrap()).unwrap();
        assert_eq!(json!(script), case["script"], "{case}");
    }

    let invalid = json!({"webrtcIP": "not an ip"});
    assert!(init_script::build_init_script(invalid.as_object().unwrap()).is_err());
}
