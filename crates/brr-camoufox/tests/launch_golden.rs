#![allow(clippy::unwrap_used)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use brr_camoufox::{
    fingerprints::{Host, IdentityInput, Pinned, TargetOs, identity_salt},
    fpgen::model::Model,
    geoip::Geo,
    launch::{LaunchInput, Machine, Position, plan_with},
    locales, pycompat,
};
use brr_core::{Error, Size};
use serde_json::Value;
use tempfile::TempDir;

const MODEL_DIR_ENV: &str = "BRR_TEST_FPGEN_MODEL";
const SCENARIO_PREFIX: &str = "scenario-";
// Launch options of the Python launcher the plan has no input for: the Profile expresses them as
// prefs, a proxy or a config the engine owns, or the host probe is not ported (R2).
const UNMAPPED_KWARGS: [&str; 17] = [
    "allow_addon_new_tab",
    "allow_webgl",
    "args",
    "block_images",
    "block_webgl",
    "canvas_noise",
    "custom_fonts_only",
    "disable_coop",
    "enable_cache",
    "fingerprint_preset",
    "geoip_db",
    "humanize",
    "main_world_eval",
    "pin_cpu_cores",
    "slow_mo",
    "timeout",
    "user_data_dir",
];
// Config overrides the plan takes as inputs: the rest of `config` has none
const MAPPED_CONFIG: [&str; 5] = [
    "timezone",
    "locale:language",
    "locale:region",
    "geolocation:latitude",
    "geolocation:longitude",
];
// Scenarios the plan can replay. A mapping change that quietly skips more fails here.
const REPLAYED_SCENARIOS: usize = 40;
// The launcher's errors a plan input can cause; the rest come from options the plan has no input for
const REACHABLE_ERRORS: [&str; 2] = ["InvalidAddonPath", "InvalidLocale"];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/launch")
}

fn load(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
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

fn fill(text: &str) -> String {
    let bundle = fixtures().join("bundle");
    let addon = fixtures().join("addons/example-addon");
    text.replace("<BUNDLE>", bundle.to_str().unwrap())
        .replace("<ADDON>", addon.to_str().unwrap())
        .replace("<HOME>", "/nonexistent-home")
}

fn target_os(name: &str) -> TargetOs {
    match name {
        "windows" => TargetOs::Windows,
        "macos" => TargetOs::Macos,
        _ => TargetOs::Linux,
    }
}

fn ff_version() -> u32 {
    let ini = fs::read_to_string(fixtures().join("bundle/application.ini")).unwrap();
    let version = ini
        .lines()
        .find_map(|line| line.strip_prefix("Version="))
        .unwrap();
    version.split('.').next().unwrap().parse().unwrap()
}

// The expected value as the plan holds it: integral floats are ints, as JavaScript cannot tell
// them apart
fn canon(value: &Value) -> Value {
    pycompat::canon(value)
}

struct Replay {
    host: Value,
    fingerprints: Value,
    model: Arc<Model>,
    work: TempDir,
}

impl Replay {
    fn geo(&self, ip: &str) -> Geo {
        let row = &self.host["geo_table"][ip];
        let country = row["country"]["iso_code"].as_str().unwrap().to_owned();
        Geo {
            ip: ip.parse().unwrap(),
            locale: locales::from_region(&country, self.host["host"]["locale_uniform"].as_f64())
                .unwrap()
                .as_string(),
            country,
            timezone: row["location"]["time_zone"].as_str().unwrap().to_owned(),
            latitude: row["location"]["latitude"].as_f64().unwrap(),
            longitude: row["location"]["longitude"].as_f64().unwrap(),
        }
    }

    // None when the scenario asks for something the plan has no input for
    fn run(&self, scenario: &Value) -> Option<Result<brr_camoufox::launch::LaunchPlan, Error>> {
        let kwargs = scenario["kwargs"].as_object().unwrap();
        let refused = scenario["error"]["type"].as_str();
        if refused.is_some_and(|kind| {
            !REACHABLE_ERRORS.contains(&kind) && !kwargs.contains_key("webgl_config")
        }) {
            return None;
        }
        let special = &scenario["special"];
        let host = &self.host["host"];

        let os = match kwargs.get("os") {
            None => None,
            Some(Value::String(name)) if ["linux", "windows", "macos"].contains(&name.as_str()) => {
                Some(target_os(name))
            }
            Some(_) => return None,
        };
        if UNMAPPED_KWARGS.iter().any(|key| kwargs.contains_key(*key))
            || special.get("host_gpu").is_some()
            || special.get("process_env").is_some()
            || special.get("random_preset").is_some()
            || kwargs.get("window").is_some()
            || kwargs["executable_path"] != "<BUNDLE>/camoufox-bin"
        {
            return None;
        }

        let pinned = match (kwargs.get("fingerprint"), special.get("generate")) {
            (Some(fingerprint), _) => Pinned {
                salt: identity_salt(fingerprint),
                fingerprint: fingerprint.clone(),
            },
            (None, Some(generate)) => Pinned {
                fingerprint: self.fingerprints[generate.as_str().unwrap()].clone(),
                salt: host["fresh_salt"].as_u64().unwrap(),
            },
            (None, None) => return None,
        };

        let env = kwargs["env"].as_object().unwrap();
        let headless = kwargs.get("headless") == Some(&Value::Bool(true));
        let virtual_display = kwargs.contains_key("virtual_display");
        let size = |width: &Value, height: &Value| Size {
            width: width.as_u64().unwrap() as u32,
            height: height.as_u64().unwrap() as u32,
        };
        // The bound on the screen the identity may claim: the caller's, else the monitor's
        let screen = match kwargs.get("screen") {
            Some(bound) => Some(size(&bound["max_width"], &bound["max_height"])),
            None if !headless
                && !virtual_display
                && (env.contains_key("DISPLAY") || env.contains_key("WAYLAND_DISPLAY")) =>
            {
                Some(size(&host["display"][0], &host["display"][1]))
            }
            None => None,
        };

        let mut addons: Vec<PathBuf> = kwargs
            .get("addons")
            .and_then(Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .map(|path| fill(path.as_str().unwrap()).into())
                    .collect()
            })
            .unwrap_or_default();
        let excluded = kwargs
            .get("exclude_addons")
            .is_some_and(|names| names == &serde_json::json!(["UBO"]));
        if !excluded {
            addons.push(fixtures().join("bundle/addons/UBO"));
        }

        let config = kwargs.get("config").and_then(Value::as_object);
        if config.is_some_and(|config| {
            config
                .keys()
                .any(|key| !MAPPED_CONFIG.contains(&key.as_str()))
        }) {
            return None;
        }
        let manual = |key: &str| config.and_then(|config| config.get(key));
        let manual_locale =
            manual("locale:language")
                .zip(manual("locale:region"))
                .map(|(language, region)| {
                    format!(
                        "{}-{}",
                        language.as_str().unwrap(),
                        region.as_str().unwrap()
                    )
                });
        let locale = kwargs.get("locale").map(|locale| match locale {
            Value::Array(list) => list
                .iter()
                .map(|item| item.as_str().unwrap())
                .collect::<Vec<_>>()
                .join(", "),
            other => other.as_str().unwrap().to_owned(),
        });
        let locale = manual_locale.or(locale);
        let geolocation = manual("geolocation:latitude")
            .zip(manual("geolocation:longitude"))
            .map(|(latitude, longitude)| Position {
                latitude: latitude.as_f64().unwrap(),
                longitude: longitude.as_f64().unwrap(),
                accuracy: None,
            });
        let geo = match kwargs.get("geoip") {
            None => None,
            Some(Value::String(ip)) => {
                if !self.host["geo_table"].as_object().unwrap().contains_key(ip) {
                    return None;
                }
                Some(self.geo(ip))
            }
            Some(_) => Some(self.geo(host["public_ip"].as_str().unwrap())),
        };

        let fonts = kwargs
            .get("fonts")
            .and_then(Value::as_array)
            .map(|fonts| {
                fonts
                    .iter()
                    .map(|font| font.as_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        let webgl = kwargs.get("webgl_config").map(|pair| {
            (
                pair[0].as_str().unwrap().to_owned(),
                pair[1].as_str().unwrap().to_owned(),
            )
        });
        let user_prefs: BTreeMap<String, Value> = kwargs
            .get("firefox_user_prefs")
            .and_then(Value::as_object)
            .map(|prefs| {
                prefs
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default();

        let bundle = fixtures().join("bundle");
        let input = LaunchInput {
            install_dir: &bundle,
            fonts_dir: &bundle.join("fonts"),
            addons: &addons,
            model: &self.model,
            identity: IdentityInput {
                os,
                screen,
                fonts,
                webgl,
                locale,
                ff_version: kwargs
                    .get("ff_version")
                    .map_or_else(ff_version, |version| version.as_u64().unwrap() as u32),
            },
            geo,
            proxied: kwargs.contains_key("proxy"),
            timezone: manual("timezone").map(|zone| zone.as_str().unwrap().to_owned()),
            geolocation,
            block_webrtc: kwargs.get("block_webrtc") == Some(&Value::Bool(true)),
            headless,
            main_world_eval: false,
            user_prefs: &user_prefs,
            seed: None,
            work_dir: self.work.path(),
        };
        let machine = Machine {
            host: Host {
                cpu_count: host["cpu_count"].as_u64().map(|count| count as u32),
                os_key: "lin",
            },
            disk_quota_kb: host["disk_capacity_kb"].as_u64(),
        };
        Some(plan_with(&input, &machine, Some(pinned)))
    }
}

fn parse_blob(blob: &str) -> Value {
    canon(&serde_json::from_str(&fill(blob)).unwrap())
}

fn assert_same_object(actual: &Value, expected: &Value, name: &str, what: &str) {
    let (actual, expected) = (actual.as_object().unwrap(), expected.as_object().unwrap());
    let differing: Vec<String> = actual
        .keys()
        .chain(expected.keys())
        .filter(|key| actual.get(*key) != expected.get(*key))
        .map(|key| {
            format!(
                "{key}: ours {:?} golden {:?}",
                actual.get(key),
                expected.get(key)
            )
        })
        .collect();
    assert!(
        differing.is_empty(),
        "{name}: {what} differs:\n{}",
        differing.join("\n")
    );
}

fn joined(env: &[(String, String)], prefix: &str) -> String {
    let mut chunks: Vec<(usize, &str)> = env
        .iter()
        .filter_map(|(key, value)| {
            let index = key.strip_prefix(prefix)?.strip_prefix('_')?;
            Some((index.parse().unwrap(), value.as_str()))
        })
        .collect();
    chunks.sort_by_key(|(index, _)| *index);
    chunks.into_iter().map(|(_, chunk)| chunk).collect()
}

#[test]
fn launch_scenarios_replay() {
    let Some(model) = model() else {
        return;
    };
    let replay = Replay {
        host: load(&fixtures().join("host.json")),
        fingerprints: load(&fixtures().join("inputs.json"))["fingerprints"].clone(),
        model,
        work: TempDir::new().unwrap(),
    };

    let mut files: Vec<PathBuf> = fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(SCENARIO_PREFIX)
        })
        .collect();
    files.sort();

    let mut replayed = 0;
    for path in files {
        let scenario = load(&path);
        let name = scenario["name"].as_str().unwrap();
        let Some(outcome) = replay.run(&scenario) else {
            continue;
        };
        replayed += 1;

        let Some(expected) = scenario.get("result") else {
            assert!(
                matches!(outcome, Err(Error::InvalidArgument { .. })),
                "{name}: the launcher refused this launch, the plan must too"
            );
            continue;
        };
        let plan = outcome.unwrap_or_else(|error| panic!("{name}: {error}"));

        let config = canon(&Value::Object(plan.config.clone()));
        assert_same_object(
            &config,
            &parse_blob(expected["config_blob_js"].as_str().unwrap()),
            name,
            "config",
        );
        assert_eq!(
            serde_json::from_str::<Value>(&joined(&plan.env, "CAMOU_CONFIG")).unwrap(),
            Value::Object(plan.config.clone()),
            "{name}: CAMOU_CONFIG chunks are not the config"
        );

        let prefs = canon(&Value::Object(plan.user_prefs.clone()));
        assert_same_object(
            &prefs,
            &parse_blob(expected["prefs_blob"].as_str().unwrap()),
            name,
            "prefs",
        );
        assert_eq!(
            canon(&serde_json::from_str(&joined(&plan.env, "CAMOU_PREFS")).unwrap()),
            prefs,
            "{name}: CAMOU_PREFS chunks are not the prefs"
        );

        for (prefix, recorded) in [
            ("CAMOU_CONFIG", "config_chunks"),
            ("CAMOU_PREFS", "prefs_chunks"),
        ] {
            let ours = plan
                .env
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .count();
            assert_eq!(
                ours,
                expected[recorded].as_array().unwrap().len(),
                "{name}: {prefix} chunk count"
            );
        }

        let fontconfig = plan
            .env
            .iter()
            .find(|(key, _)| key == "FONTCONFIG_FILE")
            .unwrap();
        assert_eq!(
            fs::read_to_string(&fontconfig.1).unwrap(),
            fill(expected["fontconfig"]["content"].as_str().unwrap()),
            "{name}: fonts.conf"
        );
    }

    assert_eq!(replayed, REPLAYED_SCENARIOS);
}
