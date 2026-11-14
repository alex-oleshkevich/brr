pub(crate) mod config;
pub mod fonts;
pub mod init_script;
pub mod media;
pub mod preset;
pub mod screen;
pub mod voices;

#[cfg(test)]
mod tests;

use brr_core::{Error, Size};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use self::{
    config::{ConfigExt, number, number_value},
    preset::set_app_version,
    screen::{HostCpu, Screen},
};
use crate::{
    coherence::{self, Config},
    data::FPGEN,
    fpgen::{
        Conditions, Constraint, generate,
        model::{Model, Ordered},
    },
    pycompat,
    pyrandom::PyRandom,
    webgl,
};

// Beyond this a font list is hostile or a mistake, not a machine
const MAX_EXTRA_FONTS: usize = 1024;
const MAX_FONT_NAME_CHARS: usize = 256;
// How much of a caller's WebGL vendor or renderer an error message echoes
const MAX_ECHOED_CHARS: usize = 64;
const SALT_BITS: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOs {
    Windows,
    Macos,
    Linux,
}

impl TargetOs {
    // The key fonts.json and the draws use
    pub fn key(self) -> &'static str {
        match self {
            TargetOs::Windows => "win",
            TargetOs::Macos => "mac",
            TargetOs::Linux => "lin",
        }
    }

    fn fpgen_name(self) -> &'static str {
        match self {
            TargetOs::Windows => "Windows",
            TargetOs::Macos => "macOS",
            TargetOs::Linux => "Linux",
        }
    }
}

#[derive(Debug, Clone)]
pub struct IdentityInput {
    pub os: Option<TargetOs>,
    pub screen: Option<Size>,
    pub fonts: Vec<String>,
    pub webgl: Option<(String, String)>,
    pub locale: Option<String>,
    pub ff_version: u32,
}

pub struct Identity {
    pub config: Map<String, Value>,
    pub webgl2_enabled: bool,
}

// What the identity reads from the machine it is drawn on, passed in so a replay can fix it
#[derive(Debug, Clone, Copy)]
pub struct Host {
    pub cpu_count: Option<u32>,
    // fonts.json's key for the host OS
    pub os_key: &'static str,
}

impl Host {
    pub fn probe() -> Host {
        Host {
            cpu_count: std::thread::available_parallelism()
                .ok()
                .and_then(|count| u32::try_from(count.get()).ok()),
            os_key: host_os_key(),
        }
    }
}

// fonts.json's key for a Camoufox OS name; anything unknown reads as macOS
fn os_key(target_os: &str) -> &'static str {
    match target_os {
        "windows" => "win",
        "linux" => "lin",
        _ => "mac",
    }
}

pub(crate) fn os_name(os_key: &str) -> &'static str {
    match os_key {
        "win" => "windows",
        "lin" => "linux",
        _ => "macos",
    }
}

// The Camoufox OS name a navigator.platform presents
fn os_name_of_platform(platform: &str) -> &'static str {
    if platform == "Win32" {
        "windows"
    } else if platform.contains("Linux") || platform.contains("linux") {
        "linux"
    } else {
        "macos"
    }
}

// The OS a user agent claims. A UA the parser does not know runs as Linux; iOS is not macOS.
pub(crate) fn os_key_from_user_agent(user_agent: &str) -> &'static str {
    let ios = ["iPhone", "iPad", "iPod"]
        .iter()
        .any(|device| user_agent.contains(device));
    if user_agent.contains("Windows") {
        "win"
    } else if !ios && (user_agent.contains("Macintosh") || user_agent.contains("Mac OS X")) {
        "mac"
    } else {
        "lin"
    }
}

fn locale_matches(locale: Option<&str>, required: &str) -> bool {
    locale
        .unwrap_or_default()
        .to_lowercase()
        .starts_with(&required.to_lowercase())
}

// sum() over floats, as CPython 3.12+ computes it (Neumaier-compensated)
fn float_sum(values: &[f64]) -> f64 {
    let Some((&first, rest)) = values.split_first() else {
        return 0.0;
    };

    let mut sum = 0.0 + first;
    let mut compensation = 0.0;
    for &value in rest {
        let total = sum + value;
        if sum.abs() >= value.abs() {
            compensation += sum - total + value;
        } else {
            compensation += value - total + sum;
        }
        sum = total;
    }
    if compensation != 0.0 && compensation.is_finite() {
        sum += compensation;
    }
    sum
}

// sum() over a mix of ints and floats, as CPython 3.14 computes it: ints add exactly until the
// first float, which is added plainly; after that every item is Neumaier-compensated
fn py_sum(values: &[Value]) -> f64 {
    let mut ints: i128 = 0;
    let mut float_sum: Option<(f64, f64)> = None;
    for value in values {
        let int = value
            .as_i64()
            .map(i128::from)
            .or_else(|| value.as_u64().map(i128::from))
            .or_else(|| value.as_bool().map(i128::from));
        match (&mut float_sum, int) {
            (None, Some(int)) => ints += int,
            (None, None) => float_sum = Some((ints as f64 + number(value), 0.0)),
            (Some((sum, compensation)), _) => {
                let x = int.map_or_else(|| number(value), |int| int as f64);
                let total = *sum + x;
                if sum.abs() >= x.abs() {
                    *compensation += *sum - total + x;
                } else {
                    *compensation += x - total + *sum;
                }
                *sum = total;
            }
        }
    }

    match float_sum {
        None => ints as f64,
        Some((sum, compensation)) if compensation != 0.0 && compensation.is_finite() => {
            sum + compensation
        }
        Some((sum, _)) => sum,
    }
}

// zlib.crc32
fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

// The entropy that makes identity_seed belong to one identity: the first 8 bytes (big-endian) of
// the SHA-256 of whatever pinned the identity -- an fpgen fingerprint, a preset, a config --
// serialized as orjson does with sorted keys, so one pinned identity draws the same fonts,
// voices, GPU and noise seeds in every launcher
pub fn identity_salt(pinned: &Value) -> u64 {
    let blob = pycompat::orjson_dumps(&pycompat::to_ordered(pinned));
    let digest = Sha256::digest(blob.as_bytes());
    u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ])
}

// The UA with the browser's version taken out, for identity_seed: a saved identity on a newer
// browser claims the new engine version and nothing else changes, so the seed must not move
// with it. Strips what follows "rv:" and "Firefox/" while it is digits and dots.
pub fn versionless_ua(user_agent: &str) -> String {
    let bytes = user_agent.as_bytes();
    let mut out = String::with_capacity(user_agent.len());
    let (mut index, mut kept) = (0, 0);
    while index < bytes.len() {
        let prefix = ["rv:", "Firefox/"]
            .into_iter()
            .find(|prefix| bytes[index..].starts_with(prefix.as_bytes()));
        if let Some(prefix) = prefix {
            let start = index + prefix.len();
            if bytes.get(start).is_some_and(u8::is_ascii_digit) {
                let mut end = start + 1;
                while bytes
                    .get(end)
                    .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
                {
                    end += 1;
                }
                out.push_str(&user_agent[kept..start]);
                kept = end;
                index = end;
                continue;
            }
        }
        index += 1;
    }
    out.push_str(&user_agent[kept..]);
    out
}

// A seed for the per-identity draws: a pure function of the presented identity (UA without its
// version, platform, screen, cores) and its salt. Not the GPU: it is sampled after the font draw.
pub fn identity_seed(config: &Config, salt: u64) -> u32 {
    let text = |key: &str| config.get(key).map_or_else(String::new, pycompat::str);
    let parts = [
        versionless_ua(&text("navigator.userAgent")),
        text("navigator.platform"),
        text("screen.width"),
        text("screen.height"),
        text("navigator.hardwareConcurrency"),
        salt.to_string(),
    ];
    crc32(parts.join("|").as_bytes())
}

// `prefix` then digits then ".0", each match's version replaced
fn replace_versions(text: &str, prefix: &str, version: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let (mut index, mut kept) = (0, 0);
    while index < bytes.len() {
        if bytes[index..].starts_with(prefix.as_bytes()) {
            let digits = index + prefix.len();
            let mut end = digits;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
            if end > digits && bytes[end..].starts_with(b".0") {
                out.push_str(&text[kept..index]);
                out.push_str(prefix);
                out.push_str(version);
                out.push_str(".0");
                kept = end + 2;
                index = kept;
                continue;
            }
        }
        index += 1;
    }
    out.push_str(&text[kept..]);
    out
}

// A three-digit "1xx.0" that is not part of a longer number, claimed as the browser's version
fn claim_version(text: &str, version: &str) -> String {
    let bytes = text.as_bytes();
    let digit = |index: usize| bytes.get(index).is_some_and(u8::is_ascii_digit);
    let mut out = String::with_capacity(text.len());
    let (mut index, mut kept) = (0, 0);
    while index < bytes.len() {
        let standalone = index == 0 || !bytes[index - 1].is_ascii_digit();
        if bytes[index] == b'1'
            && standalone
            && digit(index + 1)
            && digit(index + 2)
            && bytes[index + 3..].starts_with(b".0")
            && !digit(index + 5)
        {
            out.push_str(&text[kept..index]);
            out.push_str(version);
            out.push_str(".0");
            kept = index + 5;
            index = kept;
            continue;
        }
        index += 1;
    }
    out.push_str(&text[kept..]);
    out
}

fn to_value(value: &Ordered) -> Value {
    match value {
        Ordered::Null => Value::Null,
        Ordered::Bool(flag) => Value::Bool(*flag),
        Ordered::Number(number) => Value::Number(number.clone()),
        Ordered::String(text) => Value::String(text.clone()),
        Ordered::Array(items) => Value::Array(items.iter().map(to_value).collect()),
        Ordered::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, member)| (key.clone(), to_value(member)))
                .collect(),
        ),
    }
}

// Casts an fpgen fingerprint node onto Camoufox config properties
fn cast_to_properties(
    config: &mut Config,
    cast: &Value,
    fingerprint: &Ordered,
    ff_version: Option<&str>,
) {
    let Ordered::Object(members) = fingerprint else {
        return;
    };

    for (key, raw) in members {
        let type_key = &cast[key.as_str()];
        if !pycompat::truthy(type_key) {
            continue;
        }
        if let Ordered::Object(inner) = raw {
            if !inner.is_empty() {
                cast_to_properties(config, type_key, raw, ff_version);
            }
            continue;
        }

        let mut data = to_value(raw);
        if !pycompat::truthy(&data) {
            continue;
        }
        let Some(type_key) = type_key.as_str() else {
            continue;
        };

        // fpgen carries header values as a list; a single string is the value
        if let Value::Array(items) = &data {
            match items.as_slice() {
                [Value::String(only)] => data = Value::String(only.clone()),
                _ => continue,
            }
        }
        if type_key.starts_with("screen.")
            && (data.is_boolean() || data.is_i64() || data.is_u64())
            && number(&data) < 0.0
        {
            data = Value::from(0);
        }
        if let (Some(version), Value::String(text)) = (ff_version.filter(|v| !v.is_empty()), &data)
        {
            data = Value::String(claim_version(text, version));
        }
        config.set(type_key, data);
    }
}

// Sets window.screenY from the generated screenX value
fn handle_screen_xy(config: &mut Config, fingerprint: &Ordered, rng: &mut PyRandom) {
    if config.contains_key("window.screenY") {
        return;
    }

    let field = |node: &str, key: &str| {
        let Ordered::Object(members) = fingerprint else {
            return Value::Null;
        };
        members
            .iter()
            .find(|(name, _)| name == node)
            .and_then(|(_, value)| match value {
                Ordered::Object(inner) => inner.iter().find(|(name, _)| name == key),
                _ => None,
            })
            .map_or(Value::Null, |(_, value)| to_value(value))
    };

    let screen_x = field("window", "screenX");
    if !pycompat::truthy(&screen_x) {
        config.set("window.screenX", Value::from(0));
        config.set("window.screenY", Value::from(0));
        return;
    }
    if let Some(x) = screen_x.as_i64().filter(|x| (-50..=50).contains(x)) {
        config.set("window.screenY", Value::from(x));
        return;
    }

    // The generator thinks the browser is windowed: randomly generate a screenY
    let or_zero = |value: Value| {
        if pycompat::truthy(&value) {
            number(&value)
        } else {
            0.0
        }
    };
    let room = or_zero(field("screen", "availHeight")) - or_zero(field("window", "outerHeight"));
    let screen_y = if room == 0.0 {
        0
    } else if room > 0.0 {
        rng.randint(0, room as i64 - 1)
    } else {
        rng.randint(room as i64, -1)
    };
    config.set("window.screenY", Value::from(screen_y));
}

// Converts an fpgen fingerprint to a Camoufox config, claiming `ff_version` in its strings
pub fn from_fpgen(fingerprint: &Ordered, ff_version: Option<&str>, rng: &mut PyRandom) -> Config {
    let mut config = Config::new();
    cast_to_properties(&mut config, &FPGEN, fingerprint, ff_version);
    handle_screen_xy(&mut config, fingerprint, rng);
    set_app_version(&mut config);
    config
}

// Sets a custom outer window size and centers it in the screen
pub fn handle_window_size(fingerprint: &mut Value, outer_width: i64, outer_height: i64) {
    for node in ["screen", "window"] {
        if fingerprint[node].is_null() {
            fingerprint[node] = json!({});
        }
    }
    let or = |value: &Value, default: f64| {
        if pycompat::truthy(value) {
            number(value)
        } else {
            default
        }
    };
    let (outer_w, outer_h) = (outer_width as f64, outer_height as f64);

    let screen = fingerprint["screen"].clone();
    let window = fingerprint["window"].clone();
    let screen_x =
        or(&window["screenX"], 0.0) + ((or(&screen["width"], outer_w) - outer_w) / 2.0).floor();
    let screen_y = ((or(&screen["height"], outer_h) - outer_h) / 2.0).floor();

    let window_node = &mut fingerprint["window"];
    window_node["screenX"] = number_value(screen_x);
    window_node["screenY"] = number_value(screen_y);
    if pycompat::truthy(&window["innerWidth"]) {
        window_node["innerWidth"] = number_value(
            (outer_w - or(&window["outerWidth"], 0.0) + number(&window["innerWidth"])).max(0.0),
        );
    }
    if pycompat::truthy(&window["innerHeight"]) {
        window_node["innerHeight"] = number_value(
            (outer_h - or(&window["outerHeight"], 0.0) + number(&window["innerHeight"])).max(0.0),
        );
    }
    window_node["outerWidth"] = Value::from(outer_width);
    window_node["outerHeight"] = Value::from(outer_height);
}

fn invalid(field: &str, message: String) -> Error {
    Error::InvalidArgument {
        field: field.to_owned(),
        message,
    }
}

// The screen bound as fpgen conditions: every width and height the model holds inside it
fn screen_conditions(model: &Model, screen: &Screen) -> Result<Vec<(String, Constraint)>, Error> {
    let network = model.network();
    let node = network.node_index("screen").ok_or_else(|| {
        invalid(
            "fingerprint",
            "the fingerprint model has no screen node".to_owned(),
        )
    })?;

    let (mut widths, mut heights) = (Vec::new(), Vec::new());
    for &id in &network.nodes[node].possible {
        let text = model.lookup_text(&network.ids[id as usize])?;
        let value: Value = serde_json::from_str(&text).map_err(|err| Error::Internal {
            message: format!("decoding a fingerprint value: {err}"),
        })?;
        for (found, bounded, axis) in [
            (&mut widths, screen.allows_width(&value["width"]), "width"),
            (
                &mut heights,
                screen.allows_height(&value["height"]),
                "height",
            ),
        ] {
            if bounded == Some(true) && !found.contains(&value[axis]) {
                found.push(value[axis].clone());
            }
        }
    }

    let mut conditions = Vec::new();
    if screen.min_width.is_some() || screen.max_width.is_some() {
        conditions.push(("screen.width".to_owned(), Constraint::AnyOf(widths)));
    }
    if screen.min_height.is_some() || screen.max_height.is_some() {
        conditions.push(("screen.height".to_owned(), Constraint::AnyOf(heights)));
    }
    Ok(conditions)
}

// A Firefox fingerprint from the fpgen model. The screen bound is best-effort: a display the
// model has nothing to fit must not stop a fingerprint being generated.
pub fn generate_fingerprint(
    model: &Model,
    os: Option<TargetOs>,
    screen: Option<&Screen>,
    rng: &mut PyRandom,
) -> Result<Value, Error> {
    let base = || {
        let mut conditions = vec![("browser".to_owned(), Constraint::Equals(json!("Firefox")))];
        if let Some(os) = os {
            conditions.push(("os".to_owned(), Constraint::Equals(json!(os.fpgen_name()))));
        }
        conditions
    };

    let bounds = match screen {
        Some(screen) => screen_conditions(model, screen)?,
        None => Vec::new(),
    };
    if !bounds.is_empty() {
        let mut conditions = base();
        conditions.extend(bounds);
        match generate(model, &Conditions(conditions), rng) {
            Err(Error::InvalidArgument { field, .. }) if field == "fingerprint" => {}
            other => return other,
        }
    }
    generate(model, &Conditions(base()), rng)
}

fn host_os_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(windows) {
        "win"
    } else {
        "lin"
    }
}

fn echoed(text: &str) -> String {
    text.chars().take(MAX_ECHOED_CHARS).collect()
}

// The pair must be one fpgen recorded for Firefox on the OS: it has no WebGL parameters for any
// other, and borrowing another device's would contradict it
fn check_webgl_pair(
    model: &Model,
    os: Option<TargetOs>,
    pair: &(String, String),
) -> Result<(), Error> {
    let Some(os) = os else {
        return Err(invalid(
            "fingerprint.webgl",
            "an OS must be set to name a WebGL vendor and renderer".to_owned(),
        ));
    };
    if webgl::firefox_gpus(model, os.key())?.contains(pair) {
        return Ok(());
    }
    Err(invalid(
        "fingerprint.webgl",
        format!(
            "no recorded WebGL data for vendor {} and renderer {} from Firefox on {}",
            pycompat::str_repr(&echoed(&pair.0)),
            pycompat::str_repr(&echoed(&pair.1)),
            os.fpgen_name()
        ),
    ))
}

fn integer(config: &Config, key: &str) -> Option<i64> {
    config.get(key).and_then(Value::as_i64)
}

// The identity Camoufox claims: an fpgen fingerprint for the OS and screen bound, corrected and
// completed into the CAMOU_CONFIG keys -- navigator, screen, window, fonts, media devices, voices
// and WebGL -- and checked for coherence. Draws on `rng` for the salt that makes the seeded
// draws this identity's own, and for the model.
pub fn identity(
    model: &Model,
    input: &IdentityInput,
    rng: &mut PyRandom,
) -> Result<Map<String, Value>, Error> {
    Ok(identity_on(model, input, &Host::probe(), None, rng)?.config)
}

// An fpgen fingerprint the caller pinned, with the salt that goes with it
pub struct Pinned {
    pub fingerprint: Value,
    pub salt: u64,
}

fn screen_bound(input: &IdentityInput) -> Option<Screen> {
    input.screen.as_ref().map(|size| Screen {
        max_width: Some(i64::from(size.width)),
        max_height: Some(i64::from(size.height)),
        ..Screen::default()
    })
}

// identity() on a given machine, from a pinned fingerprint or a drawn one
pub fn identity_on(
    model: &Model,
    input: &IdentityInput,
    host: &Host,
    pinned: Option<Pinned>,
    rng: &mut PyRandom,
) -> Result<Identity, Error> {
    if let Some(pair) = &input.webgl {
        check_webgl_pair(model, input.os, pair)?;
    }
    if input.fonts.len() > MAX_EXTRA_FONTS {
        return Err(invalid(
            "fingerprint.fonts",
            format!("at most {MAX_EXTRA_FONTS} fonts, got {}", input.fonts.len()),
        ));
    }

    if let Some(font) = input
        .fonts
        .iter()
        .find(|font| font.chars().count() > MAX_FONT_NAME_CHARS)
    {
        return Err(invalid(
            "fingerprint.fonts",
            format!(
                "a font name has at most {MAX_FONT_NAME_CHARS} characters, got {}",
                font.chars().count()
            ),
        ));
    }

    let bound = screen_bound(input);
    let (fingerprint, salt) = match pinned {
        Some(Pinned { fingerprint, salt }) => (fingerprint, salt),
        None => {
            let salt = rng.getrandbits(SALT_BITS);
            (
                generate_fingerprint(model, input.os, bound.as_ref(), rng)?,
                salt,
            )
        }
    };
    let ff_version = input.ff_version.to_string();
    let mut config = from_fpgen(&pycompat::to_ordered(&fingerprint), Some(&ff_version), rng);

    let os_key = os_key_from_user_agent(&pycompat::str(config.value("navigator.userAgent")));
    let os_name = os_name(os_key);

    // A screen no desktop reports, or a netbook's: lift it to the modern floor
    if coherence::screen_is_implausible(&config) {
        coherence::repair_screen_orientation(&mut config);
        screen::raise_screen_to_modern_floor(&mut config);
    }

    screen::fix_navigator_arch(&mut config, os_key);
    let host_cpu = HostCpu {
        cpu_count: host.cpu_count,
        can_pin_host: false,
    };
    screen::fix_hardware_concurrency(&mut config, Some(false), host_cpu);

    screen::raise_screen_to_modern_floor(&mut config);
    if let Some(bound) = &bound {
        screen::clamp_screen_to_display(&mut config, bound.max_width, bound.max_height);
    }
    screen::follow_baseline_protection(&mut config, os_key);
    screen::clamp_window_dimensions(&mut config);
    screen::clamp_window_position(&mut config);

    // Fonts the caller names replace the draw, as in the launchers. Otherwise they are drawn
    // after every fix-up above, so the seed sees the final UA, screen and cores: the same
    // presented identity always gets the same font list.
    let fonts = if input.fonts.is_empty() {
        let native = (os_key == "mac" || os_key == "win") && host.os_key == os_key;
        let seed = u128::from(identity_seed(&config, salt));
        fonts::generate_random_font_subset(os_name, &mut PyRandom::seed_wide(seed), native, None)
    } else {
        input.fonts.clone()
    };
    config.set("fonts", fonts.into());

    media::set_media_devices_defaults(&mut config, salt);

    // Drawn last among the lists: the Windows voice list is the display language's pack
    let voice_locale = input
        .locale
        .clone()
        .or_else(|| config.get("navigator.language").map(pycompat::str));
    let voices = voices::generate_random_voice_subset(
        os_name,
        voice_locale.as_deref(),
        &mut PyRandom::seed_wide(u128::from(identity_seed(&config, salt))),
    )?;
    config.set("voices", Value::Array(voices));

    // Drop values the source supplied that this identity cannot keep, before the WebGL pool
    // defers to them
    coherence::drop_incoherent_source_values(&mut config, os_key);

    let seed = u128::from(identity_seed(&config, salt));
    let webgl_config = match &input.webgl {
        Some((vendor, renderer)) => webgl::for_gpu(model, os_key, vendor, renderer, Some(seed))?,
        None => webgl::sample_for_screen(
            model,
            os_key,
            integer(&config, "screen.width"),
            integer(&config, "screen.height"),
            Some(seed),
            integer(&config, "navigator.hardwareConcurrency"),
        )?,
    };
    // WebGL 2 is a browser pref, not a config key
    let mut webgl2_enabled = false;
    if let Value::Object(members) = to_value(&webgl_config) {
        for (key, value) in members {
            if key == "webGl2Enabled" {
                webgl2_enabled = value.as_bool().unwrap_or_default();
            } else {
                config.set(&key, value);
            }
        }
    }

    // The pools are sampled independently, so a machine that never existed can be assembled from
    // parts that are each fine on their own
    coherence::apply(&mut config, os_key);
    Ok(Identity {
        config,
        webgl2_enabled,
    })
}
