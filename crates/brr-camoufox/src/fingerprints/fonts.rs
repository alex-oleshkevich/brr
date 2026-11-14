use std::{collections::HashSet, path::Path};

use serde_json::Value;

use super::{float_sum, locale_matches, os_key};
use crate::{
    data::{ESSENTIAL_FONTS, FONT_BASES, FONT_GROUPS, FONTS, LAUNCHER_CONSTANTS},
    pyrandom::PyRandom,
};

static EMPTY: Vec<Value> = Vec::new();

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().unwrap_or(&EMPTY)
}

fn strings(values: &[Value]) -> impl Iterator<Item = &str> {
    values.iter().filter_map(Value::as_str)
}

fn owned(values: &[Value]) -> Vec<String> {
    strings(values).map(str::to_owned).collect()
}

// The marker fonts and the essential base per claimed OS; anything but Windows and Linux is macOS
fn essential_and_markers(target_os: &str) -> (HashSet<&'static str>, &'static [Value]) {
    let key = match target_os {
        "windows" => "win",
        "linux" => "lin",
        _ => "mac",
    };
    (
        strings(list(&ESSENTIAL_FONTS, key)).collect(),
        list(&LAUNCHER_CONSTANTS["markerFonts"], key),
    )
}

fn windows_11_markers() -> HashSet<&'static str> {
    strings(list(&LAUNCHER_CONSTANTS, "windows11MarkerFonts")).collect()
}

// SegUIVar.ttf is Segoe UI Variable: on every Windows 11 and on no Windows 10
fn host_has_variant_fonts(target_os: &str) -> bool {
    if target_os != "windows" {
        return false;
    }
    let windir = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
    Path::new(&windir)
        .join("Fonts")
        .join("SegUIVar.ttf")
        .exists()
}

// One OS-version base by its real-world weight
fn pick_base(os_key: &str, rng: &mut PyRandom) -> Vec<String> {
    let bases = list(&FONT_BASES, os_key);
    if bases.is_empty() {
        return Vec::new();
    }

    let roll = rng.random();
    let mut cumulative = 0.0;
    for base in bases {
        cumulative += base["weight"].as_f64().unwrap_or(0.0);
        if roll < cumulative {
            return owned(list(base, "fonts"));
        }
    }
    owned(list(&bases[bases.len() - 1], "fonts"))
}

// The additions this machine has, each unit judged on its own probability
fn draw_units(
    os_key: &str,
    rng: &mut PyRandom,
    exclude: &HashSet<String>,
    locale: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    for unit in list(&FONT_GROUPS, os_key) {
        if let Some(required) = unit["requiresLocale"]
            .as_str()
            .filter(|text| !text.is_empty())
            && !locale_matches(locale, required)
        {
            continue;
        }
        if rng.random() >= unit["prob"].as_f64().unwrap_or(0.0) {
            continue;
        }

        let mut members: Vec<String> = strings(list(unit, "fonts"))
            .filter(|font| !exclude.contains(*font))
            .map(str::to_owned)
            .collect();
        if members.is_empty() {
            continue;
        }

        if unit["kind"] == "alacarte" {
            let count = draw_count(unit, members.len(), rng);
            members = rng.sample(&members, count);
        }
        out.extend(members);
    }
    out
}

// How many of an a-la-carte unit's fonts a machine has: a weighted size, at least one
fn draw_count(unit: &Value, members: usize, rng: &mut PyRandom) -> usize {
    let sizes: Vec<(f64, f64)> = list(unit, "sizes")
        .iter()
        .map(|size| {
            (
                size["n"].as_f64().unwrap_or(0.0),
                size["w"].as_f64().unwrap_or(0.0),
            )
        })
        .collect();
    let sizes = if sizes.is_empty() {
        vec![(members as f64, 1.0)]
    } else {
        sizes
    };

    let weights: Vec<f64> = sizes.iter().map(|&(_, weight)| weight).collect();
    let roll = rng.random() * float_sum(&weights);
    let mut cumulative = 0.0;
    let mut count = sizes[sizes.len() - 1].0;
    for &(size, weight) in &sizes {
        cumulative += weight;
        if roll < cumulative {
            count = size;
            break;
        }
    }
    count.clamp(1.0, members as f64) as usize
}

// The font list of one plausible machine of the given OS: one OS-version base by weight, never
// subsetted, then each addition unit at its own measured probability, then the marker fonts.
// `locale` gates the units a manifest marks requiresLocale. `native` is an identity on the
// host's own macOS or Windows, where the real system fonts are used and only the base is claimed.
pub fn generate_random_font_subset(
    target_os: &str,
    rng: &mut PyRandom,
    native: bool,
    locale: Option<&str>,
) -> Vec<String> {
    let os_key = os_key(target_os);
    let full_list = FONTS[os_key]
        .as_array()
        .or_else(|| FONTS["mac"].as_array())
        .unwrap_or(&EMPTY);
    let (essential, markers) = essential_and_markers(target_os);

    if native {
        let mut result: Vec<String> = strings(full_list)
            .filter(|font| essential.contains(font))
            .map(str::to_owned)
            .collect();
        let full: HashSet<&str> = strings(full_list).collect();
        let mut missing: Vec<&str> = essential
            .iter()
            .copied()
            .filter(|font| !full.contains(font))
            .collect();
        missing.sort_unstable();
        result.extend(missing.into_iter().map(str::to_owned));

        // A native identity may run on a Windows 10 host, which cannot render the Windows 11
        // families the base carries
        if target_os == "windows" && !host_has_variant_fonts(target_os) {
            let windows_11 = windows_11_markers();
            result.retain(|font| !windows_11.contains(font.as_str()));
        }
        return result;
    }

    let mut result = pick_base(os_key, rng);
    if result.is_empty() {
        result = strings(full_list)
            .filter(|font| essential.contains(font))
            .map(str::to_owned)
            .collect();
    }
    let mut chosen: HashSet<String> = result.iter().cloned().collect();

    // The guaranteed floor underneath whichever base was drawn
    for font in strings(full_list) {
        if essential.contains(font) && !chosen.contains(font) {
            result.push(font.to_owned());
            chosen.insert(font.to_owned());
        }
    }

    // Every addition unit on its own real-world probability, atomically
    for font in draw_units(os_key, rng, &chosen, locale) {
        if chosen.insert(font.clone()) {
            result.push(font);
        }
    }

    ensure_marker_fonts(&mut result, markers);
    result
}

// Add any missing marker fonts to the font list
fn ensure_marker_fonts(fonts: &mut Vec<String>, markers: &[Value]) {
    let existing: HashSet<String> = fonts.iter().cloned().collect();
    for marker in strings(markers) {
        if !existing.contains(marker) {
            fonts.push(marker.to_owned());
        }
    }
}
