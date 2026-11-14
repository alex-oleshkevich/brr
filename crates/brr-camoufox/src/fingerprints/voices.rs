use std::{collections::HashMap, sync::LazyLock};

use brr_core::Error;
use serde_json::{Map, Value, json};

use super::{float_sum, locale_matches, os_key};
use crate::{
    data::{LAUNCHER_CONSTANTS, VOICE_MANIFESTS, VOICE_MANIFESTS_JSON, VOICE_URIS},
    fpgen::model::{Ordered, parse_ordered},
    pycompat,
    pyrandom::PyRandom,
};

const MAC_PREFIX: &str = "urn:moz-tts:osx:";
const WIN_PREFIX: &str = "urn:moz-tts:sapi:";
const LIN_PREFIX: &str = "urn:moz-tts:speechd:";
const DEFAULT_PACK_WEIGHT: f64 = 0.01;
// Latin-1 Supplement through Latin Extended-B: each character and the ASCII letters and digits
// its NFKD form holds, as the standard library has no Unicode normalization
const ASCII_FOLD: &str = include_str!("ascii_fold.txt");

static EMPTY: Vec<Value> = Vec::new();
static NULL: Value = Value::Null;

// The order of each OS's langPacks as the file writes it: the first language that shares a
// base language wins, and serde_json sorts its keys
static PACK_ORDER: LazyLock<HashMap<String, Vec<String>>> = LazyLock::new(|| {
    let mut order = HashMap::new();
    if let Ok(Ordered::Object(manifests)) = parse_ordered(VOICE_MANIFESTS_JSON) {
        for (os, manifest) in manifests {
            let Ordered::Object(members) = manifest else {
                continue;
            };
            for (key, member) in members {
                if let (true, Ordered::Object(packs)) = (key == "langPacks", member) {
                    order.insert(
                        os.clone(),
                        packs.into_iter().map(|(name, _)| name).collect(),
                    );
                }
            }
        }
    }
    order
});

fn pack<'a>(packs: &'a Map<String, Value>, key: &str) -> &'a Value {
    packs.get(key).unwrap_or(&NULL)
}

fn internal(message: String) -> Error {
    Error::Internal { message }
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value[key].as_array().unwrap_or(&EMPTY)
}

// Python truthiness of a manifest field, 0 when it is falsy: Number(x ? x : 0)
fn number_or(value: &Value, default: f64) -> f64 {
    if pycompat::truthy(value) {
        value.as_f64().unwrap_or(default)
    } else {
        default
    }
}

fn ascii_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch);
            }
        } else if let Some(entry) = ASCII_FOLD.split(' ').find(|entry| entry.starts_with(ch)) {
            out.push_str(&entry[ch.len_utf8()..]);
        }
    }
    out
}

// Stable dotted slug (shape-plausible, not catalog-exact)
fn voice_uri_slug(name: &str) -> String {
    let lowered = name.to_lowercase();
    let mut slug = String::new();
    for ch in lowered.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            slug.push(ch);
        } else if !slug.ends_with('.') {
            slug.push('.');
        }
    }
    slug.trim_matches('.').to_owned()
}

fn novelty(name: &str, key: &str) -> bool {
    LAUNCHER_CONSTANTS[key]
        .as_array()
        .is_some_and(|names| names.iter().any(|candidate| candidate == name))
}

// A voiceUri matching what real Firefox emits for the OS backend
pub fn voice_uri(os_key: &str, name: &str, lang: &str) -> String {
    match os_key {
        "lin" => {
            // SpeechDispatcherService.cpp: NS_EscapeURL(name, OnlyNonASCII|Spaces)
            let mut escaped = String::new();
            for ch in name.chars() {
                if ch == ' ' {
                    escaped.push_str("%20");
                } else if ch.is_ascii() {
                    escaped.push(ch);
                } else {
                    let mut buf = [0u8; 4];
                    for byte in ch.encode_utf8(&mut buf).bytes() {
                        escaped.push_str(&format!("%{byte:02X}"));
                    }
                }
            }
            format!("{LIN_PREFIX}{escaped}?{lang}")
        }
        // SapiService.cpp: the name and lang, verbatim
        "win" => format!("{WIN_PREFIX}{name}?{lang}"),
        "mac" => {
            if let Some(uri) = VOICE_URIS["mac"][format!("{name}|{lang}")]
                .as_str()
                .filter(|uri| !uri.is_empty())
            {
                return uri.to_owned();
            }
            let ascii = ascii_name(name);
            if novelty(name, "macNoveltyVoices") {
                format!("{MAC_PREFIX}com.apple.speech.synthesis.voice.{ascii}")
            } else if novelty(name, "macEloquenceVoices") {
                format!("{MAC_PREFIX}com.apple.eloquence.{lang}.{ascii}")
            } else {
                format!("{MAC_PREFIX}com.apple.voice.compact.{lang}.{ascii}")
            }
        }
        _ => voice_uri_slug(name),
    }
}

// "Name:lang:type" -> (name, lang, type): str.rsplit(':', 2)
fn split_entry(entry: &str) -> Result<(&str, &str, &str), Error> {
    let (rest, kind) = entry.rsplit_once(':').ok_or_else(|| {
        internal(format!(
            "not enough values to unpack (voice entry {entry:?})"
        ))
    })?;
    let (name, lang) = rest.rsplit_once(':').ok_or_else(|| {
        internal(format!(
            "not enough values to unpack (voice entry {entry:?})"
        ))
    })?;
    Ok((name, lang, kind))
}

fn voice(os_key: &str, name: &str, lang: &str, kind: &str) -> Value {
    json!({
        "name": name,
        "lang": lang,
        "voiceUri": voice_uri(os_key, name, lang),
        "isDefault": false,
        "isLocalService": kind == "local",
    })
}

// One of `items` by its "w", the way the manifests' size tables are drawn
fn weighted_pick<'a>(rng: &mut PyRandom, items: &'a [Value]) -> Option<&'a Value> {
    let weights: Vec<f64> = items
        .iter()
        .map(|item| number_or(&item["w"], 0.0))
        .collect();
    let mut x = rng.random() * float_sum(&weights);
    for (item, weight) in items.iter().zip(&weights) {
        x -= weight;
        if x <= 0.0 {
            return Some(item);
        }
    }
    items.last()
}

// How many a size table asks for: the drawn "n", truncated
fn draw_size(rng: &mut PyRandom, sizes: &[Value]) -> usize {
    weighted_pick(rng, sizes)
        .and_then(|size| size["n"].as_f64())
        .map_or(0, |n| n.trunc().max(0.0) as usize)
}

fn weighted_sample<T: Clone + PartialEq>(
    rng: &mut PyRandom,
    items: &[T],
    k: usize,
    weight: impl Fn(&T) -> f64,
) -> Vec<T> {
    let mut pool = items.to_vec();
    let mut out = Vec::new();
    while !pool.is_empty() && out.len() < k {
        let weights: Vec<f64> = pool.iter().map(&weight).collect();
        let mut x = rng.random() * float_sum(&weights);
        let mut picked = None;
        for (index, weight) in weights.iter().enumerate() {
            x -= weight;
            if x <= 0.0 {
                picked = Some(index);
                break;
            }
        }

        match picked {
            Some(index) => {
                let item = pool[index].clone();
                let first = pool.iter().position(|candidate| *candidate == item);
                pool.remove(first.unwrap_or(index));
                out.push(item);
            }
            None => out.extend(pool.pop()),
        }
    }
    out
}

// The language pack a display locale selects: the locale itself, else the first pack of its
// base language, else the fallback, else the first pack
fn resolve_display_pack(
    order: &[String],
    packs: &Map<String, Value>,
    fallback: &str,
    locale: Option<&str>,
) -> String {
    if let Some(locale) = locale.filter(|locale| !locale.is_empty()) {
        if packs.contains_key(locale) {
            return locale.to_owned();
        }
        let lang = locale.split('-').next().unwrap_or_default().to_lowercase();
        if let Some(key) = order
            .iter()
            .find(|key| key.split('-').next().unwrap_or_default().to_lowercase() == lang)
        {
            return key.clone();
        }
    }
    if packs.contains_key(fallback) {
        fallback.to_owned()
    } else {
        order.first().cloned().unwrap_or_default()
    }
}

struct Draw {
    out: Vec<String>,
    legacy: Vec<String>,
}

impl Draw {
    fn add(&mut self, entries: &[Value]) {
        for entry in entries.iter().filter_map(Value::as_str) {
            if !self.out.iter().any(|existing| existing == entry) {
                self.out.push(entry.to_owned());
            }
        }
    }

    fn take_pack(&mut self, rng: &mut PyRandom, pack: &Value) {
        self.add(list(pack, "oneCore"));
        if pycompat::truthy(&pack["desktop"]) && rng.random() < number_or(&pack["desktopProb"], 0.0)
        {
            for entry in list(pack, "desktop").iter().filter_map(Value::as_str) {
                if !self.legacy.iter().any(|existing| existing == entry) {
                    self.legacy.push(entry.to_owned());
                }
            }
        }
    }

    // A downloaded voice sits in its alphabetical place
    fn insert_sorted(&mut self, entry: &str) {
        if self.out.iter().any(|existing| existing == entry) {
            return;
        }
        let lowered = entry.to_lowercase();
        let index = self
            .out
            .iter()
            .position(|existing| existing.to_lowercase() > lowered)
            .unwrap_or(self.out.len());
        self.out.insert(index, entry.to_owned());
    }
}

// The speech voice list for the given OS as MaskConfig voice objects, following the measured
// per-OS model in voice-manifests.json. No voice is marked default: stock Firefox marks none.
pub fn generate_random_voice_subset(
    target_os: &str,
    locale: Option<&str>,
    rng: &mut PyRandom,
) -> Result<Vec<Value>, Error> {
    let os_key = os_key(target_os);
    let manifest = if pycompat::truthy(&VOICE_MANIFESTS[os_key]) {
        &VOICE_MANIFESTS[os_key]
    } else {
        &VOICE_MANIFESTS["mac"]
    };
    if manifest.is_null() {
        return Err(internal("no voice manifest for mac".to_owned()));
    }
    let order = PACK_ORDER
        .get(if pycompat::truthy(&VOICE_MANIFESTS[os_key]) {
            os_key
        } else {
            "mac"
        })
        .map_or(&[][..], Vec::as_slice);

    let mut draw = Draw {
        out: Vec::new(),
        legacy: Vec::new(),
    };
    let empty_packs = Map::new();
    let packs = if pycompat::truthy(&manifest["langPacks"]) {
        manifest["langPacks"].as_object().unwrap_or(&empty_packs)
    } else {
        &empty_packs
    };

    draw.add(list(manifest, "base"));
    let mut chosen: Vec<String> = Vec::new();
    if !packs.is_empty() {
        let fallback = manifest["fallbackLocale"]
            .as_str()
            .filter(|text| !text.is_empty())
            .unwrap_or("en-US");
        let key = resolve_display_pack(order, packs, fallback, locale);
        draw.take_pack(rng, pack(packs, &key));
        chosen.push(key);
    }

    for addition in list(manifest, "additions") {
        if pycompat::truthy(&addition["deferred"]) {
            continue;
        }
        if let Some(required) = addition["requiresLocale"]
            .as_str()
            .filter(|text| !text.is_empty())
            && !locale_matches(locale, required)
        {
            continue;
        }
        if rng.random() >= number_or(&addition["prob"], 0.0) {
            continue;
        }

        match addition["kind"].as_str() {
            Some("bundle") => draw.add(list(addition, "voices")),
            Some("alacarte") => {
                let sizes = if pycompat::truthy(&addition["sizes"]) {
                    list(addition, "sizes").to_vec()
                } else {
                    vec![json!({"n": 1, "w": 1})]
                };
                let k = draw_size(rng, &sizes);
                let voices: Vec<&str> = list(addition, "voices")
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                for entry in weighted_sample(rng, &voices, k, |_| 1.0) {
                    draw.insert_sorted(entry);
                }
            }
            Some("groups") if !packs.is_empty() => {
                let eligible: Vec<&str> = list(addition, "groups")
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|group| {
                        packs.contains_key(*group) && !chosen.iter().any(|done| done == group)
                    })
                    .collect();
                if eligible.is_empty() {
                    continue;
                }

                let k = if pycompat::truthy(&addition["sizes"]) {
                    draw_size(rng, list(addition, "sizes"))
                } else {
                    eligible.len()
                };
                let weight =
                    |group: &&str| number_or(&pack(packs, group)["weight"], DEFAULT_PACK_WEIGHT);
                for group in weighted_sample(rng, &eligible, k, weight) {
                    chosen.push(group.to_owned());
                    draw.take_pack(rng, pack(packs, group));
                }
            }
            _ => {}
        }
    }

    draw.out
        .iter()
        .chain(&draw.legacy)
        .map(|entry| {
            let (name, lang, kind) = split_entry(entry)?;
            Ok(voice(os_key, name, lang, kind))
        })
        .collect()
}

// Coerce a preset's speechVoices into MaskConfig voice objects: presets store "Name:lang:type"
// strings, which MaskConfig drops. Objects pass through.
pub fn normalize_preset_voices(voices: &[Value], target_os: &str) -> Vec<Value> {
    let os_key = os_key(target_os);
    let mut result = Vec::new();
    for entry in voices {
        let Some(text) = entry.as_str() else {
            if entry.is_object() {
                result.push(entry.clone());
            }
            continue;
        };
        let Some((before, kind)) = text.rsplit_once(':') else {
            continue;
        };
        let Some((name, lang)) = before.rsplit_once(':') else {
            continue;
        };
        if name.is_empty() || lang.is_empty() {
            continue;
        }
        result.push(voice(os_key, name, lang, kind));
    }

    let any_default = result
        .iter()
        .any(|voice| pycompat::truthy(&voice["isDefault"]));
    if !result.is_empty() && !any_default {
        result[0]["isDefault"] = Value::Bool(true);
    }
    result
}
