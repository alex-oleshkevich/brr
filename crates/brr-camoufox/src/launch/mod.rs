#[cfg(test)]
mod tests;

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use brr_core::{Error, Size};
use nix::sys::statvfs::statvfs;
use serde_json::{Map, Value};

use crate::{
    coherence::Config,
    data::LAUNCHER_CONSTANTS,
    fingerprints::{
        self, Host, IdentityInput, Pinned,
        config::{ConfigExt, int_value},
        os_key_from_user_agent, os_name,
    },
    fpgen::model::Model,
    geoip::Geo,
    locales, pycompat,
    pyrandom::PyRandom,
};

pub const CHUNK_CODE_POINTS: usize = 32_767;

const CONFIG_PREFIX: &str = "CAMOU_CONFIG";
const PREFS_PREFIX: &str = "CAMOU_PREFS";
const FONTCONFIG_ENV: &str = "FONTCONFIG_FILE";
const FONTCONFIG_NAME: &str = "fonts.conf";
const PROPERTIES_NAME: &str = "properties.json";
const GROUPS_NAME: &str = "groups.json";
const MANIFEST_NAME: &str = "manifest.json";
// The line of the bundled fonts.conf that names the fonts directory relative to the working dir
const FONTS_DIR_LINE: &str = "<dir prefix=\"cwd\">fonts</dir>";
const FONT_DIR_SEPARATOR: &str = "\n\t";
const BYTES_PER_KB: u64 = 1024;
const WINDOWS_OS_KEY: &str = "win";
const LINUX_OS_KEY: &str = "lin";
const MAC_OS_KEY: &str = "mac";
const APPLE_VENDOR: &str = "Apple";
const DEFAULT_UI_LOCALE: &str = "en-US";
// Gecko's own value for "render at the system's scale"
const SCROLLBAR_STYLE_WINDOWS_11: i64 = 5;
const SCROLLBAR_STYLE_WINDOWS_10: i64 = 4;
// widget/GfxInfoFeatureStatusDefs.inc: a blocked device resolves requestAdapter() to null
const WEBGPU_BLOCKED_DEVICE: i64 = 4;
const BASELINE_OVERRIDES_PREF: &str = "privacy.baselineFingerprintingProtection.overrides";
// The canvas targets privacy.resistFingerprinting turns on, and only those
const CANVAS_PLACEHOLDER_TARGETS: [&str; 3] = [
    "+CanvasImageExtractionPrompt",
    "+CanvasExtractionBeforeUserInputIsBlocked",
    "+CanvasExtractionFromThirdPartiesIsBlocked",
];
const ARGS: [&str; 2] = ["-no-remote", "-juggler-pipe"];
const HEADLESS_ARG: &str = "-headless";

pub struct LaunchInput<'a> {
    // The Build directory: properties.json and fontconfig/
    pub install_dir: &'a Path,
    pub fonts_dir: &'a Path,
    // Extracted add-on directories, each with a manifest.json
    pub addons: &'a [PathBuf],
    pub model: &'a Model,
    pub identity: IdentityInput,
    pub geo: Option<Geo>,
    // The browser reaches the network through a proxy
    pub proxied: bool,
    // A time zone and a position the Profile sets by hand; the exit IP's position wins
    pub timezone: Option<String>,
    pub geolocation: Option<Position>,
    // WebRTC is off: no address is spoofed for it
    pub block_webrtc: bool,
    pub headless: bool,
    // Lets `mw:`-prefixed scripts run in the page's own world; off, only the isolated world exists
    pub main_world_eval: bool,
    pub user_prefs: &'a BTreeMap<String, Value>,
    pub seed: Option<u64>,
    // Where the generated fonts.conf is written
    pub work_dir: &'a Path,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: Option<f64>,
}

pub struct LaunchPlan {
    pub env: Vec<(String, String)>,
    pub args: Vec<OsString>,
    pub user_prefs: Map<String, Value>,
    pub config: Map<String, Value>,
    pub window: Option<Size>,
}

// Test seam: what a plan reads from the machine, passed in so a replay can fix it
#[derive(Debug, Clone, Copy)]
pub struct Machine {
    pub host: Host,
    pub disk_quota_kb: Option<u64>,
}

impl Machine {
    // `disk_dir` names the disk whose capacity the browser's storage quota reads
    pub fn probe(disk_dir: &Path) -> Machine {
        Machine {
            host: Host::probe(),
            disk_quota_kb: disk_quota_kb(disk_dir),
        }
    }
}

// Gecko reports a quota of min(capacity / 10, 10 GiB), so any disk of 100 GB or more reads back
// the same and clamping the signed 32-bit KB pref changes nothing a page can see.
fn disk_quota_kb(dir: &Path) -> Option<u64> {
    let stats = statvfs(dir).ok()?;
    let total = stats.blocks() * stats.fragment_size();
    (total > 0).then(|| (total / 2 / BYTES_PER_KB).min(i32::MAX as u64))
}

fn invalid(field: &str, message: String) -> Error {
    Error::InvalidArgument {
        field: field.to_owned(),
        message,
    }
}

fn build_missing(reason: String) -> Error {
    Error::BuildMissing { reason }
}

pub fn plan(input: &LaunchInput) -> Result<LaunchPlan, Error> {
    plan_with(input, &Machine::probe(input.fonts_dir), None)
}

// Test seam: plan() on a given machine, optionally from an fpgen fingerprint the caller pinned
pub fn plan_with(
    input: &LaunchInput,
    machine: &Machine,
    pinned: Option<Pinned>,
) -> Result<LaunchPlan, Error> {
    let mut rng = input.seed.map_or_else(PyRandom::from_os, PyRandom::seed);

    // The exit IP's locale first, then the caller's over it: handle_locales overwrites the
    // keys it writes and leaves the others as they were
    let mut locale_config = Config::new();
    if let Some(geo) = &input.geo {
        locales::handle_locales(&geo.locale, &mut locale_config)?;
    }
    if let Some(locale) = &input.identity.locale {
        locales::handle_locales(locale, &mut locale_config)?;
    }

    // The voice list is the display language's pack, so it is drawn for the resolved locale
    let mut identity_input = input.identity.clone();
    identity_input.locale = voice_locale(&locale_config);

    let identity = fingerprints::identity_on(
        input.model,
        &identity_input,
        &machine.host,
        pinned,
        &mut rng,
    )?;
    let mut config = identity.config;
    let os_key = os_key_from_user_agent(&pycompat::str(config.value("navigator.userAgent")));

    if !input.addons.is_empty() {
        config.set("addons", confirm_addons(input.addons)?);
    }

    let mut prefs: Map<String, Value> = input
        .user_prefs
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    set_launcher_prefs(&mut prefs, &mut config, os_key, machine);

    if let Some(timezone) = &input.timezone {
        config.set("timezone", timezone.clone().into());
    }
    if let Some(position) = &input.geolocation {
        config.set("geolocation:latitude", position.latitude.into());
        config.set("geolocation:longitude", position.longitude.into());
        if let Some(accuracy) = position.accuracy.filter(|accuracy| *accuracy != 0.0) {
            config.set("geolocation:accuracy", accuracy.into());
        }
    }
    if let Some(geo) = &input.geo {
        apply_geo(&mut config, geo, input.proxied, input.block_webrtc);
    }
    config.extend(locale_config);
    if input.main_world_eval {
        set_into(&mut config, "allowMainWorld", true.into());
    }
    set_into(&mut config, "voices:blockIfNotDefined", true.into());
    if config.contains_key("geolocation:latitude") && config.contains_key("geolocation:longitude") {
        set_into(&mut prefs, "permissions.default.geo", 1.into());
    }

    let requested = if config.truthy("locale:language") {
        ["locale:language", "locale:script", "locale:region"]
            .iter()
            .map(|key| config.value(key))
            .filter(|part| pycompat::truthy(part))
            .map(pycompat::str)
            .collect::<Vec<_>>()
            .join("-")
    } else {
        DEFAULT_UI_LOCALE.to_owned()
    };
    set_into(&mut prefs, "intl.locale.requested", requested.into());

    set_into(&mut prefs, "permissions.default.image", 1.into());
    set_into(
        &mut prefs,
        "media.peerconnection.enabled",
        (!input.block_webrtc).into(),
    );
    set_into(
        &mut prefs,
        "browser.tabs.remote.useCrossOriginOpenerPolicy",
        true.into(),
    );

    // The canvas renders on this machine. The identity never claims its GPU, so it only agrees
    // with the pixels on the host's own OS, and a GPU the caller named is another machine's.
    if os_key != machine.host.os_key || input.identity.webgl.is_some() {
        add_canvas_placeholder(&mut prefs);
    }
    set_into(
        &mut prefs,
        "webgl.enable-webgl2",
        identity.webgl2_enabled.into(),
    );
    set_into(&mut prefs, "webgl.force-enabled", true.into());

    // Render at the claimed display scale, so the CSS geometry agrees with devicePixelRatio
    let dpr = config.number("window.devicePixelRatio");
    if dpr != 0.0 && dpr != 1.0 {
        set_into(
            &mut prefs,
            "layout.css.devPixelsPerPx",
            dpr.to_string().into(),
        );
    }
    set_webgpu(&mut prefs, os_key, &config);

    validate_config(&config, input.install_dir)?;

    let mut env = chunk_env(CONFIG_PREFIX, &config_json(&config));
    env.push((
        FONTCONFIG_ENV.to_owned(),
        generate_fontconfig(input, os_key)?,
    ));
    env.extend(chunk_env(PREFS_PREFIX, &prefs_json(&prefs)));

    let mut args: Vec<OsString> = ARGS.iter().map(OsString::from).collect();
    if input.headless {
        args.push(HEADLESS_ARG.into());
    }

    Ok(LaunchPlan {
        env,
        args,
        window: window_size(&config),
        user_prefs: prefs,
        config,
    })
}

fn voice_locale(locale_config: &Config) -> Option<String> {
    let language = locale_config
        .truthy("locale:language")
        .then(|| pycompat::str(locale_config.value("locale:language")))?;
    Some(if locale_config.truthy("locale:region") {
        format!(
            "{language}-{}",
            pycompat::str(locale_config.value("locale:region"))
        )
    } else {
        language
    })
}

fn set_into(target: &mut Map<String, Value>, key: &str, value: Value) {
    target.entry(key.to_owned()).or_insert(value);
}

// The prefs and config keys the launcher derives from the claimed OS and machine; anything the
// profile already set is kept
fn set_launcher_prefs(
    prefs: &mut Map<String, Value>,
    config: &mut Config,
    os_key: &str,
    machine: &Machine,
) {
    // Scrollbars: the look-and-feel of the claimed OS, so headless, headed and stock agree. For
    // Windows, the version the identity's fonts present: Windows 11 fonts with classic
    // scrollbars is a pair no real machine produces.
    let overlay = if os_key == WINDOWS_OS_KEY {
        let fonts = config.get("fonts").and_then(Value::as_array);
        let marker_fonts = LAUNCHER_CONSTANTS["windows11MarkerFonts"].as_array();
        let windows_11 = fonts.is_none_or(|fonts| {
            fonts.is_empty()
                || fonts
                    .iter()
                    .any(|font| marker_fonts.is_some_and(|markers| markers.contains(font)))
        });
        i64::from(windows_11)
    } else {
        1
    };
    set_into(prefs, "ui.useOverlayScrollbars", overlay.into());

    // A Windows identity on another host: answers that come from the host build's platform code
    // are set to what stock Windows Firefox gives
    if os_key == WINDOWS_OS_KEY && machine.host.os_key != WINDOWS_OS_KEY {
        set_into(prefs, "javascript.options.use_ucrt_math", true.into());
        set_into(config, "input:windowsKeyboard", true.into());
        if let Some(behaviour) = LAUNCHER_CONSTANTS["windowsBehaviourPrefs"].as_object() {
            for (pref, value) in behaviour {
                set_into(prefs, pref, value.clone());
            }
        }

        // The host build's theme draws scrollbars 12 px wide; Windows' own are 4 (Windows 10)
        // or 5 (Windows 11)
        let overlay_on =
            pycompat::truthy(prefs.get("ui.useOverlayScrollbars").unwrap_or(&Value::Null));
        let style = if overlay_on {
            SCROLLBAR_STYLE_WINDOWS_11
        } else {
            SCROLLBAR_STYLE_WINDOWS_10
        };
        set_into(
            prefs,
            "widget.non-native-theme.scrollbar.style",
            style.into(),
        );
    }

    // Per-character font fallback, Linux only: with async fallback on, the first measurement of a
    // character only one bundled family provides returns the primary family's .notdef
    if os_key == LINUX_OS_KEY {
        set_into(prefs, "gfx.font_rendering.fallback.async", false.into());
    }

    if let Some(quota) = machine.disk_quota_kb {
        set_into(
            prefs,
            "dom.quotaManager.temporaryStorage.fixedLimit",
            quota.into(),
        );
    }

    // Do Not Track and Global Privacy Control are pref-backed in Firefox, and a stock Firefox
    // reports "unspecified" and false: the generated values are dropped, not spoofed
    config.remove("navigator.doNotTrack");
    config.remove("navigator.globalPrivacyControl");
    prefs.insert("privacy.donottrackheader.enabled".to_owned(), false.into());
    prefs.insert(
        "privacy.globalprivacycontrol.enabled".to_owned(),
        false.into(),
    );

    // Firefox's own Accept-Encoding is already what the identity claims
    config.remove("headers.Accept-Encoding");
}

// What the exit IP says about where the browser is: the position, the time zone, and the WebRTC
// address
fn apply_geo(config: &mut Config, geo: &Geo, proxied: bool, block_webrtc: bool) {
    if !block_webrtc {
        let webrtc = if geo.ip.is_ipv4() {
            "webrtc:ipv4"
        } else {
            "webrtc:ipv6"
        };
        set_into(config, webrtc, geo.ip.to_string().into());
    }
    set_into(config, "timezone", geo.timezone.clone().into());
    config.set("geolocation:longitude", geo.longitude.into());
    config.set("geolocation:latitude", geo.latitude.into());

    // Without a proxy a site that has an IPv6 address would see this host's, not the IPv4
    // address the identity was drawn from
    if !proxied && geo.ip.is_ipv4() {
        config.set("network:ipv4Only", true.into());
    }
}

fn add_canvas_placeholder(prefs: &mut Map<String, Value>) {
    let current = prefs
        .get(BASELINE_OVERRIDES_PREF)
        .map(pycompat::str)
        .unwrap_or_default();
    let mut overrides: Vec<&str> = current.split(',').filter(|part| !part.is_empty()).collect();
    overrides.extend(CANVAS_PLACEHOLDER_TARGETS);
    prefs.insert(
        BASELINE_OVERRIDES_PREF.to_owned(),
        overrides.join(",").into(),
    );
}

// Firefox enables WebGPU on Windows and on Apple Silicon Macs only. Any other adapter would
// contradict the claimed GPU, so the device is blocked and requestAdapter() resolves to null.
// A Profile that set the pref itself is the one whose value decides.
fn set_webgpu(prefs: &mut Map<String, Value>, os_key: &str, config: &Config) {
    let enabled = os_key == WINDOWS_OS_KEY
        || (os_key == MAC_OS_KEY && config.value("webGl:vendor") == APPLE_VENDOR);
    set_into(prefs, "dom.webgpu.enabled", enabled.into());

    if pycompat::truthy(&prefs["dom.webgpu.enabled"]) {
        set_into(prefs, "gfx.blacklist.webgpu", WEBGPU_BLOCKED_DEVICE.into());
    }
}

fn confirm_addons(addons: &[PathBuf]) -> Result<Value, Error> {
    for addon in addons {
        if !addon.join(MANIFEST_NAME).is_file() {
            return Err(invalid(
                "extensions",
                format!(
                    "{} is not an extracted add-on: it has no {MANIFEST_NAME}",
                    addon.display()
                ),
            ));
        }
    }
    Ok(addons
        .iter()
        .map(|addon| Value::from(addon.to_string_lossy().into_owned()))
        .collect())
}

// The window the browser opens at, when the identity spoofs one
fn window_size(config: &Config) -> Option<Size> {
    let dimension = |key: &str| u32::try_from(int_value(config.value(key))?).ok();
    Some(Size {
        width: dimension("window.outerWidth")?,
        height: dimension("window.outerHeight")?,
    })
}

// A config value against the type the Build declares for the key. A key the Build does not know
// is skipped: it is for a newer or older browser and has no effect.
fn validate_config(config: &Config, install_dir: &Path) -> Result<(), Error> {
    let path = install_dir.join(PROPERTIES_NAME);
    let text = fs::read_to_string(&path).map_err(|error| {
        build_missing(format!(
            "cannot read {}: {error}; install the Camoufox Build again",
            path.display()
        ))
    })?;
    let properties: Vec<Value> = serde_json::from_str(&text).map_err(|error| {
        build_missing(format!(
            "{} is not a property list: {error}; install the Camoufox Build again",
            path.display()
        ))
    })?;

    for (key, value) in config {
        let Some(entry) = properties.iter().find(|entry| entry["property"] == *key) else {
            continue;
        };
        if entry.get("removed").is_some() {
            continue;
        }
        let expected = entry["type"].as_str().unwrap_or_default();
        if !is_type(value, expected) {
            return Err(invalid(
                "fingerprint",
                format!("{key} must be {expected}, got {}", pycompat::repr(value)),
            ));
        }
    }
    Ok(())
}

// A boolean passes the numeric checks, as in the launchers; a float passes int when integral
fn is_type(value: &Value, expected: &str) -> bool {
    let integral = match value {
        Value::Bool(_) => true,
        Value::Number(number) => number.as_f64().is_some_and(|float| float.fract() == 0.0),
        _ => false,
    };
    match expected {
        "str" => value.is_string(),
        "int" => integral,
        "uint" => integral && number_of(value) >= 0.0,
        "double" => value.is_number() || value.is_boolean(),
        "bool" => value.is_boolean(),
        "array" => value.is_array(),
        "dict" => value.is_object(),
        _ => false,
    }
}

fn number_of(value: &Value) -> f64 {
    match value {
        Value::Bool(flag) => f64::from(*flag),
        other => other.as_f64().unwrap_or_default(),
    }
}

// The bundled fonts.conf with the font directories this OS reads, written to the work directory.
// fontconfig scans a directory recursively, so the parent is never named: that would make every
// other OS's fonts reachable. The bundle stores each face once, in a directory named for the set
// of OSes that use it, and an OS reads the groups its letter is in.
fn generate_fontconfig(input: &LaunchInput, os_key: &str) -> Result<String, Error> {
    let os_dir = os_name(os_key);
    let source = input
        .install_dir
        .join("fontconfig")
        .join(os_dir)
        .join(FONTCONFIG_NAME);
    let template = fs::read_to_string(&source).map_err(|error| {
        build_missing(format!(
            "cannot read {}: {error}; install the Camoufox Build again",
            source.display()
        ))
    })?;

    let groups_path = input.fonts_dir.join(GROUPS_NAME);
    let groups: Value = fs::read_to_string(&groups_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| {
            build_missing(format!(
                "cannot read the font groups {}; install the fonts bundle again",
                groups_path.display()
            ))
        })?;
    let scan_dirs: Vec<String> = groups["readBy"][os_key]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(|group| input.fonts_dir.join(group))
        .filter(|dir| dir.is_dir())
        .map(|dir| Ok(format!("<dir>{}</dir>", xml_escape(utf8(&dir)?))))
        .collect::<Result<_, Error>>()?;
    if scan_dirs.is_empty() {
        return Err(build_missing(format!(
            "the fonts bundle {} has no fonts for {os_dir}; install the fonts bundle again",
            input.fonts_dir.display()
        )));
    }

    let content = template.replace(FONTS_DIR_LINE, &scan_dirs.join(FONT_DIR_SEPARATOR));
    let target = input.work_dir.join(FONTCONFIG_NAME);
    fs::create_dir_all(input.work_dir)
        .and_then(|()| fs::write(&target, content))
        .map_err(|error| Error::Internal {
            message: format!("writing {}: {error}", target.display()),
        })?;
    Ok(utf8(&target)?.to_owned())
}

// fontconfig reads these from XML and the environment, which cannot carry a path that is not text
fn utf8(path: &Path) -> Result<&str, Error> {
    path.to_str().ok_or_else(|| Error::Internal {
        message: format!("{} is not valid UTF-8", path.display()),
    })
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;")
}

// The config as orjson writes it: compact, UTF-8
fn config_json(config: &Config) -> String {
    pycompat::orjson_dumps(&pycompat::to_ordered(&Value::Object(config.clone())))
}

// The prefs as Python's json.dumps(ensure_ascii=True) writes them. Only ASCII reaches the
// environment: on Windows autoconfig reads it through the ANSI code page, where a raw UTF-8
// value is mangled and \u escapes survive.
fn prefs_json(prefs: &Map<String, Value>) -> String {
    let compact = config_json(prefs);
    let mut ascii = String::with_capacity(compact.len());
    for ch in compact.chars() {
        if ch.is_ascii() && ch != '\u{7f}' {
            ascii.push(ch);
        } else {
            let mut units = [0u16; 2];
            for unit in ch.encode_utf16(&mut units) {
                ascii.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    ascii
}

// `value` in chunks of CHUNK_CODE_POINTS code points, as NAME_1..NAME_N. The browser joins them in
// numeric order, and a chunk never ends inside a character.
pub fn chunk_env(prefix: &str, value: &str) -> Vec<(String, String)> {
    let mut chunks = Vec::new();
    let (mut start, mut count) = (0, 0);
    for (index, _) in value.char_indices() {
        if count == CHUNK_CODE_POINTS {
            chunks.push(&value[start..index]);
            start = index;
            count = 0;
        }
        count += 1;
    }
    if start < value.len() {
        chunks.push(&value[start..]);
    }

    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| (format!("{prefix}_{}", index + 1), chunk.to_owned()))
        .collect()
}
