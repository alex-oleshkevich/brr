use serde_json::Value;

use super::config::{ConfigExt, int_value, number, number_value};
use crate::coherence::Config;
use crate::{data::LAUNCHER_CONSTANTS, pycompat};

pub const MODERN_SCREEN_FLOOR: (f64, f64) = (1366.0, 768.0);

// A bound on the screen a generated fingerprint may claim
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Screen {
    pub min_width: Option<i64>,
    pub max_width: Option<i64>,
    pub min_height: Option<i64>,
    pub max_height: Option<i64>,
}

fn within(value: &Value, low: Option<i64>, high: Option<i64>) -> bool {
    value
        .as_number()
        .and_then(serde_json::Number::as_i64)
        .is_some_and(|value| {
            low.is_none_or(|low| value >= low) && high.is_none_or(|high| value <= high)
        })
}

impl Screen {
    // The fpgen condition on screen.width: None when the width is unbounded
    pub fn allows_width(&self, value: &Value) -> Option<bool> {
        (self.min_width.is_some() || self.max_width.is_some())
            .then(|| within(value, self.min_width, self.max_width))
    }

    pub fn allows_height(&self, value: &Value) -> Option<bool> {
        (self.min_height.is_some() || self.max_height.is_some())
            .then(|| within(value, self.min_height, self.max_height))
    }
}

fn constant(key: &str, os_key: &str) -> f64 {
    LAUNCHER_CONSTANTS[key][os_key].as_f64().unwrap_or(0.0)
}

// What Firefox's baseline fingerprinting protection reports in place of the device's values, and
// the browser applies it to the spoofed ones for the claimed OS: the available screen is the
// whole screen less a fixed taskbar, and more than one touch point reads as 5. Claim both here;
// clamp_window_dimensions then fits the window inside that screen.
pub fn follow_baseline_protection(config: &mut Config, os_key: &str) {
    if config.truthy("screen.width") && config.truthy("screen.height") {
        let width = config.value("screen.width").clone();
        let height = config.number("screen.height");
        config.set("screen.availWidth", width);
        config.set(
            "screen.availHeight",
            number_value(height - constant("baselineTaskbarHeight", os_key)),
        );
    }

    let touch_points = config.get("navigator.maxTouchPoints").map_or(0.0, number);
    if touch_points > 1.0 {
        config.set(
            "navigator.maxTouchPoints",
            LAUNCHER_CONSTANTS["baselineMultiTouchPoints"].clone(),
        );
    }
}

fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(pycompat::truthy)
}

fn num(value: Option<&Value>) -> f64 {
    value.map_or(f64::NAN, number)
}

// Enforce inner <= outer <= avail <= screen on both axes
pub fn clamp_window_dimensions(config: &mut Config) {
    for axis in ["Width", "Height"] {
        let screen_key = format!("screen.{}", axis.to_lowercase());
        let avail_key = format!("screen.avail{axis}");
        let outer_key = format!("window.outer{axis}");
        let inner_key = format!("window.inner{axis}");
        let screen = config.get(&screen_key).cloned();
        let avail = config.get(&avail_key).cloned();
        let outer = config.get(&outer_key).cloned();
        let inner = config.get(&inner_key).cloned();

        // avail must not exceed screen
        if truthy(screen.as_ref())
            && truthy(avail.as_ref())
            && num(avail.as_ref()) > num(screen.as_ref())
            && let Some(screen) = &screen
        {
            config.set(&avail_key, screen.clone());
        }
        let avail_clamped = config.get(&avail_key).cloned().or_else(|| screen.clone());

        // outer must not exceed avail (or screen if avail is unknown)
        let outer_cap = match &avail_clamped {
            Some(Value::Null) | None => screen.clone(),
            Some(avail) => Some(avail.clone()),
        };
        if truthy(outer.as_ref())
            && truthy(outer_cap.as_ref())
            && num(outer.as_ref()) > num(outer_cap.as_ref())
            && let Some(cap) = &outer_cap
        {
            let chrome = if truthy(inner.as_ref()) {
                (num(outer.as_ref()) - num(inner.as_ref())).max(0.0)
            } else {
                0.0
            };
            config.set(&outer_key, cap.clone());
            if truthy(inner.as_ref()) {
                config.set(&inner_key, number_value((number(cap) - chrome).max(1.0)));
            }
        }

        // inner must not exceed outer
        let outer_clamped = config.get(&outer_key).cloned().or(outer);
        let inner_now = config.get(&inner_key).cloned();
        if truthy(inner_now.as_ref())
            && truthy(outer_clamped.as_ref())
            && num(inner_now.as_ref()) > num(outer_clamped.as_ref())
            && let Some(outer) = outer_clamped
        {
            config.set(&inner_key, outer);
        }
    }
}

// Shrink screen.width/height down to the bounds of the real display, keeping the taskbar delta.
// Run clamp_window_dimensions afterwards.
pub fn clamp_screen_to_display(
    config: &mut Config,
    max_width: Option<i64>,
    max_height: Option<i64>,
) {
    for (axis, avail_key, cap) in [
        ("width", "screen.availWidth", max_width),
        ("height", "screen.availHeight", max_height),
    ] {
        let screen_key = format!("screen.{axis}");
        let Some(cap) = cap.filter(|&cap| cap != 0) else {
            continue;
        };
        if !config.truthy(&screen_key) || config.number(&screen_key) <= cap as f64 {
            continue;
        }

        let screen = config.number(&screen_key);
        let avail = config.get(avail_key).cloned();
        config.set(&screen_key, Value::from(cap));
        if let Some(avail) = avail.filter(pycompat::truthy) {
            let gap = (screen - number(&avail)).max(0.0);
            config.set(avail_key, number_value((cap as f64 - gap).max(1.0)));
        }
    }
}

// Keep the window box inside the screen: 0 <= screenX/Y <= screen - outer
pub fn clamp_window_position(config: &mut Config) {
    for (axis, position_key) in [("Width", "window.screenX"), ("Height", "window.screenY")] {
        let screen_key = format!("screen.{}", axis.to_lowercase());
        let outer_key = format!("window.outer{axis}");
        let position = config.value(position_key);
        if position.is_null() || !config.truthy(&screen_key) || !config.truthy(&outer_key) {
            continue;
        }

        let room = config.number(&screen_key) - config.number(&outer_key);
        let clamped = number(position).min(room).max(0.0);
        config.set(position_key, number_value(clamped));
    }
}

// Lift netbook-era screen geometry to the modern floor, keeping the screen-to-avail gaps. Call
// before clamp_screen_to_display.
pub fn raise_screen_to_modern_floor(config: &mut Config) {
    let (min_width, min_height) = MODERN_SCREEN_FLOOR;
    if !config.truthy("screen.width") || !config.truthy("screen.height") {
        return;
    }
    let (width, height) = (
        config.number("screen.width"),
        config.number("screen.height"),
    );
    if width >= min_width && height >= min_height {
        return;
    }

    let gap = |avail_key: &str, full: f64| {
        config
            .truthy(avail_key)
            .then(|| full - config.number(avail_key))
    };
    let (gap_width, gap_height) = (
        gap("screen.availWidth", width),
        gap("screen.availHeight", height),
    );

    let (new_width, new_height) = (width.max(min_width), height.max(min_height));
    config.set("screen.width", number_value(new_width));
    config.set("screen.height", number_value(new_height));
    if let Some(gap) = gap_width {
        config.set(
            "screen.availWidth",
            number_value((new_width - gap.max(0.0)).max(1.0)),
        );
    }
    if let Some(gap) = gap_height {
        config.set(
            "screen.availHeight",
            number_value((new_height - gap.max(0.0)).max(1.0)),
        );
    }
}

// The logical CPUs the browser's host offers, and whether it can pin a process to cores
#[derive(Debug, Clone, Copy)]
pub struct HostCpu {
    pub cpu_count: Option<u32>,
    pub can_pin_host: bool,
}

fn plausible_core_counts() -> Vec<i64> {
    LAUNCHER_CONSTANTS["plausibleCoreCounts"]
        .as_array()
        .map(|counts| counts.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

// The largest plausible core count up to `limit`, else the smallest
fn snap_down(counts: &[i64], limit: i64) -> i64 {
    counts
        .iter()
        .copied()
        .rfind(|&count| count <= limit)
        .or_else(|| counts.first().copied())
        .unwrap_or(limit)
}

// navigator.hardwareConcurrency = the host's parallelism, snapped down into the plausible core
// counts, or the fingerprint's own value (snapped, capped by the host) when the browser can be
// pinned to that many cores. `can_pin` is false when nothing will pin the browser.
pub fn fix_hardware_concurrency(config: &mut Config, can_pin: Option<bool>, host: HostCpu) {
    let Some(cpus) = host.cpu_count.filter(|&count| count != 0) else {
        return;
    };
    let pinnable = host.can_pin_host && can_pin != Some(false);
    let counts = plausible_core_counts();
    let cap = i64::from(cpus);

    if pinnable
        && let Some(drawn) =
            int_value(config.value("navigator.hardwareConcurrency")).filter(|&drawn| drawn >= 1)
    {
        config.set(
            "navigator.hardwareConcurrency",
            Value::from(snap_down(&counts, drawn.min(cap))),
        );
        return;
    }
    config.set(
        "navigator.hardwareConcurrency",
        Value::from(snap_down(&counts, cap)),
    );
}

// Force navigator.platform and navigator.oscpu to match the UA's arch (Linux)
pub fn fix_navigator_arch(config: &mut Config, os_key: &str) {
    if os_key != "lin" || !config.truthy("navigator.userAgent") {
        return;
    }

    let user_agent = pycompat::str(config.value("navigator.userAgent"));
    let target = if user_agent.contains("Linux x86_64") {
        "Linux x86_64"
    } else if user_agent.contains("Linux i686") {
        "Linux i686"
    } else {
        return;
    };
    config.set("navigator.platform", Value::from(target));
    config.set("navigator.oscpu", Value::from(target));
}
