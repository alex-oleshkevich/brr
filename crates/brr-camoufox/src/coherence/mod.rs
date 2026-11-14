use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

use serde_json::{Map, Value};

use crate::{data::LAUNCHER_CONSTANTS, pycompat};

pub type Config = Map<String, Value>;

const MAX_PLAUSIBLE_TOUCH_POINTS: i64 = 10;
const BROWSER_CHROME_HEIGHT: f64 = 86.0;
const PLAUSIBLE_COLOR_DEPTHS: [f64; 2] = [24.0, 30.0];
const DEEP_COLOR_DEPTH: f64 = 30.0;
const DEFAULT_COLOR_DEPTH: f64 = 24.0;
const MIN_DESKTOP_WIDTH: f64 = 1024.0;
// Firefox 152 has no ANGLE on a Mac. "Intel(R) HD Graphics 400" and "Radeon R9 200 Series" are
// the buckets an Intel Mac's UHD 630 and Radeon Pro 5300M report.
const NOT_A_MAC_GPU: [&str; 2] = ["ANGLE", "llvmpipe"];

const RENDERER: &str = "webGl:renderer";
const VENDOR: &str = "webGl:vendor";
const CORES: &str = "navigator.hardwareConcurrency";
const COLOR_DEPTH: &str = "screen.colorDepth";
const PIXEL_DEPTH: &str = "screen.pixelDepth";
const TOUCH_POINTS: &str = "navigator.maxTouchPoints";
const DEVICE_PIXEL_RATIO: &str = "window.devicePixelRatio";
const INNER_HEIGHT: &str = "window.innerHeight";
const OUTER_HEIGHT: &str = "window.outerHeight";
const SCREEN_WIDTH: &str = "screen.width";
const SCREEN_HEIGHT: &str = "screen.height";
const AVAIL_WIDTH: &str = "screen.availWidth";
const AVAIL_HEIGHT: &str = "screen.availHeight";
const USER_AGENT: &str = "navigator.userAgent";
const PLATFORM: &str = "navigator.platform";
const OSCPU: &str = "navigator.oscpu";

static NULL: Value = Value::Null;

struct Constants {
    apple_silicon_cores: Vec<i64>,
    intel_igp_cores: HashSet<i64>,
    intel_dgpu_cores: HashSet<i64>,
    apple_silicon_panels: HashSet<(i64, i64)>,
    plausible_dpr: HashMap<String, Vec<f64>>,
}

static CONSTANTS: LazyLock<Constants> = LazyLock::new(|| {
    let integers = |key: &str| -> Vec<i64> {
        LAUNCHER_CONSTANTS[key]
            .as_array()
            .map(|items| items.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default()
    };

    let mut apple_silicon_cores = integers("appleSiliconCores");
    apple_silicon_cores.sort_unstable();
    let intel_igp_cores: HashSet<i64> = integers("intelMacIgpCores").into_iter().collect();
    let mut intel_dgpu_cores = intel_igp_cores.clone();
    intel_dgpu_cores.extend(integers("intelMacDgpuExtraCores"));

    let apple_silicon_panels = LAUNCHER_CONSTANTS["appleSiliconPanels"]
        .as_object()
        .into_iter()
        .flat_map(|models| models.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|panel| Some((panel[0].as_i64()?, panel[1].as_i64()?)))
        .collect();

    // Ascending: the nearest-step repair keeps the first of two equally near steps
    let plausible_dpr = LAUNCHER_CONSTANTS["plausibleDpr"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(os, steps)| {
            let steps = steps.as_array().into_iter().flatten();
            (os.clone(), steps.filter_map(Value::as_f64).collect())
        })
        .collect();

    Constants {
        apple_silicon_cores,
        intel_igp_cores,
        intel_dgpu_cores,
        apple_silicon_panels,
        plausible_dpr,
    }
});

#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub rule: &'static str,
    pub detail: String,
}

type Check = fn(&Config, &str) -> Option<String>;
type Repair = fn(&mut Config, &str);

struct Rule {
    name: &'static str,
    check: Check,
    // None where no correct value is determined
    repair: Option<Repair>,
}

const RULES: [Rule; 10] = [
    Rule {
        name: "apple-silicon-cores",
        check: check_apple_silicon_cores,
        repair: Some(repair_apple_silicon_cores),
    },
    Rule {
        name: "gpu-matches-os",
        check: check_gpu_matches_os,
        repair: None,
    },
    Rule {
        name: "intel-mac-hardware",
        check: check_intel_mac_hardware,
        repair: None,
    },
    Rule {
        name: "color-depth",
        check: check_color_depth,
        repair: Some(repair_color_depth),
    },
    Rule {
        name: "touch-points",
        check: check_touch_points,
        repair: Some(repair_touch_points),
    },
    Rule {
        name: "device-pixel-ratio",
        check: check_device_pixel_ratio,
        repair: Some(repair_device_pixel_ratio),
    },
    Rule {
        name: "window-chrome",
        check: check_window_chrome,
        repair: Some(repair_window_chrome),
    },
    Rule {
        name: "screen-shape",
        check: check_screen_shape,
        repair: None,
    },
    Rule {
        name: "avail-bounds",
        check: check_avail_bounds,
        repair: Some(repair_avail_bounds),
    },
    Rule {
        name: "arch-agreement",
        check: check_arch_agreement,
        repair: None,
    },
];

fn get<'a>(config: &'a Config, key: &str) -> &'a Value {
    config.get(key).unwrap_or(&NULL)
}

fn truthy(config: &Config, key: &str) -> bool {
    pycompat::truthy(get(config, key))
}

// JavaScript's Number(): what the TypeScript twin compares and subtracts with
fn number(value: &Value) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(flag) => f64::from(*flag),
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::String(text) if text.trim().is_empty() => 0.0,
        Value::String(text) => text.trim().parse().unwrap_or(f64::NAN),
        Value::Array(_) | Value::Object(_) => f64::NAN,
    }
}

fn number_value(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < pycompat::SAFE_INTEGER_LIMIT {
        Value::from(value as i64)
    } else {
        Value::from(value)
    }
}

// isinstance(value, int): bool counts, a float does not
fn int_value(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number.as_i64(),
        _ => None,
    }
}

fn renderer(config: &Config) -> String {
    let value = get(config, RENDERER);
    if pycompat::truthy(value) {
        pycompat::str(value)
    } else {
        String::new()
    }
}

fn is_apple_silicon(config: &Config) -> bool {
    renderer(config).contains("Apple M")
}

fn check_apple_silicon_cores(config: &Config, _target_os: &str) -> Option<String> {
    if !is_apple_silicon(config) {
        return None;
    }

    let cores = get(config, CORES);
    let count = int_value(cores)?;
    if CONSTANTS.apple_silicon_cores.contains(&count) {
        return None;
    }
    Some(format!(
        "{} with hardwareConcurrency {}; Apple Silicon starts at 8",
        pycompat::str_repr(&renderer(config)),
        pycompat::str(cores)
    ))
}

fn repair_apple_silicon_cores(config: &mut Config, _target_os: &str) {
    let Some(count) = int_value(get(config, CORES)) else {
        return;
    };

    let steps = &CONSTANTS.apple_silicon_cores;
    let snapped = steps
        .iter()
        .find(|&&step| step >= count)
        .or(steps.last())
        .copied();
    if let Some(snapped) = snapped {
        config.insert(CORES.to_owned(), Value::from(snapped));
    }
}

// Whether this renderer string is one the OS can report. Used both to check a finished identity
// and to filter the WebGL pool before sampling.
pub fn gpu_fits_os(renderer: Option<&str>, target_os: &str) -> bool {
    let renderer = renderer.unwrap_or_default();
    if renderer.is_empty() {
        return true;
    }
    match target_os {
        "mac" => !NOT_A_MAC_GPU.iter().any(|bad| renderer.contains(bad)),
        "win" => renderer.starts_with("ANGLE"),
        "lin" => !renderer.contains("ANGLE") && !renderer.contains("Apple M"),
        _ => true,
    }
}

fn is_apple_silicon_panel(width: &Value, height: &Value) -> bool {
    let (Value::Number(width), Value::Number(height)) = (width, height) else {
        return false;
    };
    let (Some(width), Some(height)) = (width.as_f64(), height.as_f64()) else {
        return false;
    };
    width.fract() == 0.0
        && height.fract() == 0.0
        && CONSTANTS
            .apple_silicon_panels
            .contains(&(width as i64, height as i64))
}

// Why no Intel Mac reports this GPU beside these cores and this screen
fn intel_mac_misfit(
    renderer: &str,
    cores: &Value,
    width: &Value,
    height: &Value,
) -> Option<String> {
    if renderer.is_empty() || renderer.contains("Apple M") {
        return None;
    }

    let allowed = if renderer.contains("Intel") {
        &CONSTANTS.intel_igp_cores
    } else {
        &CONSTANTS.intel_dgpu_cores
    };
    if int_value(cores).is_some_and(|count| !allowed.contains(&count)) {
        return Some(format!(
            "{} with hardwareConcurrency {}; no Intel Mac with that GPU reports it",
            pycompat::str_repr(renderer),
            pycompat::str(cores)
        ));
    }
    if is_apple_silicon_panel(width, height) {
        return Some(format!(
            "{} behind a {}x{} panel, which only Apple Silicon Macs have",
            pycompat::str_repr(renderer),
            pycompat::str(width),
            pycompat::str(height)
        ));
    }
    None
}

// Whether a machine with this GPU has these cores and this screen. Only Intel Macs are modelled;
// serves the WebGL draw and the finished-identity check.
pub fn gpu_fits_machine(
    renderer: Option<&str>,
    target_os: &str,
    cores: &Value,
    width: &Value,
    height: &Value,
) -> bool {
    target_os != "mac"
        || intel_mac_misfit(renderer.unwrap_or_default(), cores, width, height).is_none()
}

fn check_intel_mac_hardware(config: &Config, target_os: &str) -> Option<String> {
    if target_os != "mac" {
        return None;
    }
    intel_mac_misfit(
        &renderer(config),
        get(config, CORES),
        get(config, SCREEN_WIDTH),
        get(config, SCREEN_HEIGHT),
    )
}

fn check_gpu_matches_os(config: &Config, target_os: &str) -> Option<String> {
    let renderer = renderer(config);
    if renderer.is_empty() || gpu_fits_os(Some(&renderer), target_os) {
        return None;
    }

    let renderer = pycompat::str_repr(&renderer);
    Some(match target_os {
        "mac" => format!("macOS identity with {renderer}, which no Mac reports"),
        "win" => {
            format!("Windows identity with {renderer}; Firefox on Windows renders through ANGLE")
        }
        _ => format!("Linux identity with {renderer}"),
    })
}

fn in_color_depths(depth: &Value) -> bool {
    matches!(depth, Value::Number(depth) if PLAUSIBLE_COLOR_DEPTHS.contains(&depth.as_f64().unwrap_or(f64::NAN)))
}

fn check_color_depth(config: &Config, target_os: &str) -> Option<String> {
    let depth = get(config, COLOR_DEPTH);
    if depth.is_null() {
        return None;
    }
    if !in_color_depths(depth) {
        return Some(format!(
            "screen.colorDepth {}; Firefox reports 24 or 30",
            pycompat::str(depth)
        ));
    }
    if target_os == "mac" && is_apple_silicon(config) && number(depth) != DEEP_COLOR_DEPTH {
        return Some(format!(
            "Apple Silicon Mac with colorDepth {}; deep colour is the macOS default",
            pycompat::str(depth)
        ));
    }
    None
}

fn repair_color_depth(config: &mut Config, target_os: &str) {
    let depth = get(config, COLOR_DEPTH);
    if !depth.is_null() && !in_color_depths(depth) {
        config.insert(COLOR_DEPTH.to_owned(), number_value(DEFAULT_COLOR_DEPTH));
    }
    if target_os == "mac" && is_apple_silicon(config) {
        config.insert(COLOR_DEPTH.to_owned(), number_value(DEEP_COLOR_DEPTH));
    }

    // pixelDepth is the same number in every browser that reports both
    if config.contains_key(PIXEL_DEPTH) || config.contains_key(COLOR_DEPTH) {
        let depth = config
            .get(COLOR_DEPTH)
            .cloned()
            .unwrap_or_else(|| number_value(DEFAULT_COLOR_DEPTH));
        config.insert(PIXEL_DEPTH.to_owned(), depth);
    }
}

fn check_touch_points(config: &Config, target_os: &str) -> Option<String> {
    let touch = get(config, TOUCH_POINTS);
    if touch.is_null() {
        return None;
    }

    if !int_value(touch).is_some_and(|count| (0..=MAX_PLAUSIBLE_TOUCH_POINTS).contains(&count)) {
        return Some(format!(
            "navigator.maxTouchPoints {}; a digitiser reports at most {MAX_PLAUSIBLE_TOUCH_POINTS}",
            pycompat::str(touch)
        ));
    }
    if target_os == "mac" && pycompat::truthy(touch) {
        return Some(format!(
            "macOS identity with maxTouchPoints {}; no Mac has a touchscreen",
            pycompat::str(touch)
        ));
    }
    None
}

fn repair_touch_points(config: &mut Config, target_os: &str) {
    let touch = get(config, TOUCH_POINTS);
    let too_many = int_value(touch).is_some_and(|count| count > MAX_PLAUSIBLE_TOUCH_POINTS);
    if target_os == "mac" || too_many || (!touch.is_null() && int_value(touch).is_none()) {
        config.insert(TOUCH_POINTS.to_owned(), Value::from(0));
    }
}

fn plausible_dpr(target_os: &str) -> Option<&'static [f64]> {
    CONSTANTS
        .plausible_dpr
        .get(target_os)
        .map(Vec::as_slice)
        .filter(|steps| !steps.is_empty())
}

fn check_device_pixel_ratio(config: &Config, target_os: &str) -> Option<String> {
    let dpr = get(config, DEVICE_PIXEL_RATIO);
    if dpr.is_null() {
        return None;
    }

    let allowed = plausible_dpr(target_os)?;
    if allowed.contains(&number(dpr)) {
        return None;
    }
    Some(format!(
        "window.devicePixelRatio {} is not a display mode {target_os} offers",
        pycompat::str(dpr)
    ))
}

fn repair_device_pixel_ratio(config: &mut Config, target_os: &str) {
    let dpr = get(config, DEVICE_PIXEL_RATIO);
    let Some(allowed) = plausible_dpr(target_os) else {
        return;
    };
    if dpr.is_null() {
        return;
    }

    // The nearest real scaling step; the first of equal distances wins
    let wanted = number(dpr);
    let mut best = allowed[0];
    for &step in &allowed[1..] {
        if (step - wanted).abs() < (best - wanted).abs() {
            best = step;
        }
    }
    config.insert(DEVICE_PIXEL_RATIO.to_owned(), number_value(best));
}

fn check_window_chrome(config: &Config, _target_os: &str) -> Option<String> {
    if !truthy(config, INNER_HEIGHT) || !truthy(config, OUTER_HEIGHT) {
        return None;
    }

    let (inner, outer) = (get(config, INNER_HEIGHT), get(config, OUTER_HEIGHT));
    let chrome = number(outer) - number(inner);
    if chrome >= BROWSER_CHROME_HEIGHT || chrome.is_nan() {
        return None;
    }
    Some(format!(
        "window.outerHeight {} - innerHeight {} = {}, less than the {}px of chrome the window \
         actually has; the bottom {}px of the claimed viewport cannot receive input",
        pycompat::str(outer),
        pycompat::str(inner),
        pycompat::str(&number_value(chrome)),
        pycompat::str(&number_value(BROWSER_CHROME_HEIGHT)),
        pycompat::str(&number_value(BROWSER_CHROME_HEIGHT - chrome))
    ))
}

fn repair_window_chrome(config: &mut Config, _target_os: &str) {
    if !truthy(config, INNER_HEIGHT) || !truthy(config, OUTER_HEIGHT) {
        return;
    }

    let inner = number(get(config, INNER_HEIGHT));
    let outer = number(get(config, OUTER_HEIGHT));
    let avail = if truthy(config, AVAIL_HEIGHT) {
        get(config, AVAIL_HEIGHT)
    } else {
        get(config, SCREEN_HEIGHT)
    };

    // Prefer growing the window, which keeps the viewport the identity drew
    if !pycompat::truthy(avail) || inner + BROWSER_CHROME_HEIGHT <= number(avail) {
        config.insert(
            OUTER_HEIGHT.to_owned(),
            number_value(inner + BROWSER_CHROME_HEIGHT),
        );
        return;
    }
    // No room on the claimed screen: shrink the viewport instead
    config.insert(
        INNER_HEIGHT.to_owned(),
        number_value((outer - BROWSER_CHROME_HEIGHT).max(1.0)),
    );
}

fn check_screen_shape(config: &Config, _target_os: &str) -> Option<String> {
    if !truthy(config, SCREEN_WIDTH) || !truthy(config, SCREEN_HEIGHT) {
        return None;
    }

    let (width, height) = (get(config, SCREEN_WIDTH), get(config, SCREEN_HEIGHT));
    if number(height) > number(width) {
        return Some(format!(
            "portrait screen {}x{}; desktop panels are landscape",
            pycompat::str(width),
            pycompat::str(height)
        ));
    }
    if number(width) < MIN_DESKTOP_WIDTH {
        return Some(format!(
            "screen {}x{} is smaller than any current desktop panel",
            pycompat::str(width),
            pycompat::str(height)
        ));
    }
    None
}

fn check_avail_bounds(config: &Config, _target_os: &str) -> Option<String> {
    let exceeds = |size_key: &str, avail_key: &str| {
        truthy(config, size_key)
            && truthy(config, avail_key)
            && number(get(config, avail_key)) > number(get(config, size_key))
    };

    if exceeds(SCREEN_WIDTH, AVAIL_WIDTH) {
        return Some(format!(
            "screen.availWidth {} exceeds screen.width {}",
            pycompat::str(get(config, AVAIL_WIDTH)),
            pycompat::str(get(config, SCREEN_WIDTH))
        ));
    }
    if exceeds(SCREEN_HEIGHT, AVAIL_HEIGHT) {
        return Some(format!(
            "screen.availHeight {} exceeds screen.height {}",
            pycompat::str(get(config, AVAIL_HEIGHT)),
            pycompat::str(get(config, SCREEN_HEIGHT))
        ));
    }
    None
}

fn repair_avail_bounds(config: &mut Config, _target_os: &str) {
    for (size_key, avail_key) in [(SCREEN_WIDTH, AVAIL_WIDTH), (SCREEN_HEIGHT, AVAIL_HEIGHT)] {
        let size = get(config, size_key);
        // A missing avail value reads as 0 here
        if pycompat::truthy(size) && number(get(config, avail_key)) > number(size) {
            let size = size.clone();
            config.insert(avail_key.to_owned(), size);
        }
    }
}

fn check_arch_agreement(config: &Config, _target_os: &str) -> Option<String> {
    let text = |key: &str| {
        if truthy(config, key) {
            pycompat::str(get(config, key))
        } else {
            String::new()
        }
    };
    let user_agent = text(USER_AGENT);
    let platform = text(PLATFORM);
    let oscpu = text(OSCPU);

    if user_agent.contains("x86_64") && (platform.contains("armv") || oscpu.contains("armv")) {
        return Some(format!(
            "user agent claims x86_64 while platform/oscpu say {}/{}",
            pycompat::str_repr(&platform),
            pycompat::str_repr(&oscpu)
        ));
    }
    None
}

// Whether the screen is one no desktop reports (portrait, or tiny)
pub fn screen_is_implausible(config: &Config) -> bool {
    check_screen_shape(config, "").is_some()
}

// Turn a portrait screen landscape, keeping the panel's own dimensions. Run before the window
// clamps, which then bound the window to the new screen.
pub fn repair_screen_orientation(config: &mut Config) -> bool {
    if !truthy(config, SCREEN_WIDTH)
        || !truthy(config, SCREEN_HEIGHT)
        || number(get(config, SCREEN_HEIGHT)) <= number(get(config, SCREEN_WIDTH))
    {
        return false;
    }

    swap(config, SCREEN_WIDTH, SCREEN_HEIGHT);
    if truthy(config, AVAIL_WIDTH) && truthy(config, AVAIL_HEIGHT) {
        swap(config, AVAIL_WIDTH, AVAIL_HEIGHT);
    }
    true
}

fn swap(config: &mut Config, first: &str, second: &str) {
    let first_value = get(config, first).clone();
    let second_value = get(config, second).clone();
    config.insert(first.to_owned(), second_value);
    config.insert(second.to_owned(), first_value);
}

// Discard values a source supplied that this identity cannot keep (a preset's GPU pair its OS,
// or its final core count and screen, rule out), so the normal WebGL sampling draws a coherent
// one instead. Run after the core count and the screen are final.
pub fn drop_incoherent_source_values(config: &mut Config, target_os: &str) -> Vec<Violation> {
    let renderer = renderer(config);
    if renderer.is_empty() {
        return Vec::new();
    }

    let rule = if !gpu_fits_os(Some(&renderer), target_os) {
        "gpu-matches-os"
    } else if check_intel_mac_hardware(config, target_os).is_some() {
        "intel-mac-hardware"
    } else {
        return Vec::new();
    };
    config.remove(RENDERER);
    config.remove(VENDOR);
    vec![Violation {
        rule,
        detail: format!(
            "dropped {} for a {target_os} identity",
            pycompat::str_repr(&renderer)
        ),
    }]
}

// Every invariant this identity breaks. Empty means coherent.
pub fn validate(config: &Config, target_os: &str) -> Vec<Violation> {
    RULES
        .iter()
        .filter_map(|rule| {
            (rule.check)(config, target_os).map(|detail| Violation {
                rule: rule.name,
                detail,
            })
        })
        .collect()
}

// Repair what is determined, and report what is left. A rule with no repair cannot be corrected
// without inventing a machine, so it is returned for the caller to decide about.
pub fn apply(config: &mut Config, target_os: &str) -> Vec<Violation> {
    for rule in &RULES {
        if let Some(repair) = rule.repair
            && (rule.check)(config, target_os).is_some()
        {
            repair(config, target_os);
        }
    }
    validate(config, target_os)
}

#[cfg(test)]
mod tests;
