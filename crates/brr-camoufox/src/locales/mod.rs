mod suppress_script;

#[cfg(test)]
mod tests;

use brr_core::Error;
use serde_json::Value;

use self::suppress_script::SUPPRESS_SCRIPT;
use crate::{coherence::Config, data::TERRITORY_INFO};

const MAX_SUBTAG_CHARS: usize = 8;
const SCRIPT_CHARS: usize = 4;
const MAX_REGION_ALPHA_CHARS: usize = 2;
const NUMERIC_REGION_CHARS: usize = 3;
const MIN_VARIANT_CHARS: usize = 5;
// A region or a language code is at most this long; anything longer is a full tag
const MAX_BARE_CODE_CHARS: usize = 3;
const PERCENT_DIVISOR: f64 = 100.0;
const PAIRWISE_BLOCK: usize = 8;
const PAIRWISE_UNROLL_MAX: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    pub language: String,
    pub region: Option<String>,
    pub script: Option<String>,
}

impl Locale {
    pub fn as_string(&self) -> String {
        match &self.region {
            Some(region) => format!("{}-{region}", self.language),
            None => self.language.clone(),
        }
    }

    // The locale:* keys Camoufox reads
    pub fn write_config(&self, config: &mut Config) {
        if let Some(region) = &self.region {
            config.insert("locale:region".to_owned(), region.clone().into());
        }
        config.insert("locale:language".to_owned(), self.language.clone().into());
        if let Some(script) = &self.script {
            config.insert("locale:script".to_owned(), script.clone().into());
        }
    }
}

fn invalid(message: String) -> Error {
    Error::InvalidArgument {
        field: "locale".to_owned(),
        message,
    }
}

fn invalid_locale(locale: &str) -> Error {
    invalid(format!(
        "{locale:?} is not a locale; use a region, a language, language-region or language-script-region, such as en-US"
    ))
}

fn territories() -> &'static [Value] {
    TERRITORY_INFO.as_array().map_or(&[], Vec::as_slice)
}

fn base_language(code: &str) -> &str {
    code.split('_').next().unwrap_or(code)
}

fn language_is_known(language: &str) -> bool {
    SUPPRESS_SCRIPT
        .binary_search_by_key(&language, |(code, _)| code)
        .is_ok()
        || territories().iter().any(|territory| {
            languages_of(territory)
                .iter()
                .any(|entry| base_language(entry["type"].as_str().unwrap_or_default()) == language)
        })
}

fn languages_of(territory: &Value) -> &[Value] {
    territory["languages"].as_array().map_or(&[], Vec::as_slice)
}

fn suppress_script(language: &str) -> Option<&'static str> {
    SUPPRESS_SCRIPT
        .binary_search_by_key(&language, |(code, _)| code)
        .ok()
        .map(|index| SUPPRESS_SCRIPT[index].1)
}

// BCP 47 language[-script][-region][-variant...] reduced to what Camoufox reads. The script
// is the language's Suppress-Script, not the tag's own: that is what the launchers do, so
// en-US is en/Latn/US and zh-Hans-CN is zh/CN.
pub fn normalize_locale(locale: &str) -> Result<Locale, Error> {
    let subtags: Vec<&str> = locale.split('-').collect();
    if subtags
        .iter()
        .any(|tag| tag.is_empty() || tag.len() > MAX_SUBTAG_CHARS || !tag.is_ascii())
        || subtags
            .iter()
            .any(|tag| !tag.chars().all(|c| c.is_ascii_alphanumeric()))
    {
        return Err(invalid_locale(locale));
    }

    let language = subtags[0].to_ascii_lowercase();
    let is_language = (2..=MAX_BARE_CODE_CHARS).contains(&language.len())
        && language.chars().all(|c| c.is_ascii_lowercase());
    if !is_language || !language_is_known(&language) {
        return Err(invalid_locale(locale));
    }

    let mut rest = subtags[1..].iter().peekable();
    let is_alpha = |tag: &str| tag.chars().all(|c| c.is_ascii_alphabetic());
    // The tag's own script is skipped, see above
    let _ = rest.next_if(|tag| tag.len() == SCRIPT_CHARS && is_alpha(tag));
    let region = rest
        .next_if(|tag| {
            (tag.len() == MAX_REGION_ALPHA_CHARS && is_alpha(tag))
                || (tag.len() == NUMERIC_REGION_CHARS && tag.chars().all(|c| c.is_ascii_digit()))
        })
        .map(|tag| tag.to_ascii_uppercase());
    // An alphabetic region must be a territory: en-UK is not a locale, en-GB is
    if region.as_ref().is_some_and(|region| {
        region.chars().all(|c| c.is_ascii_alphabetic())
            && !territories()
                .iter()
                .any(|territory| territory["type"] == *region)
    }) {
        return Err(invalid_locale(locale));
    }
    while rest
        .next_if(|tag| {
            tag.len() >= MIN_VARIANT_CHARS
                || (tag.len() == SCRIPT_CHARS && tag.starts_with(|c: char| c.is_ascii_digit()))
        })
        .is_some()
    {}
    if rest.next().is_some() {
        return Err(invalid_locale(locale));
    }

    let Some(region) = region else {
        return Err(invalid_locale(locale));
    };
    Ok(Locale {
        script: suppress_script(&language).map(str::to_owned),
        language,
        region: Some(region),
    })
}

// numpy's pairwise summation, so the normalized probabilities match the launchers' to the last bit
pub fn pairwise_sum(values: &[f64]) -> f64 {
    let len = values.len();
    if len < PAIRWISE_BLOCK {
        return values.iter().fold(0.0, |sum, value| sum + value);
    }
    if len <= PAIRWISE_UNROLL_MAX {
        let mut lanes = [0.0; PAIRWISE_BLOCK];
        lanes.copy_from_slice(&values[..PAIRWISE_BLOCK]);
        let blocked = len - len % PAIRWISE_BLOCK;
        for block in values[PAIRWISE_BLOCK..blocked].chunks_exact(PAIRWISE_BLOCK) {
            for (lane, value) in lanes.iter_mut().zip(block) {
                *lane += value;
            }
        }
        let mut sum = lanes[0]
            + lanes[1]
            + (lanes[2] + lanes[3])
            + (lanes[4] + lanes[5] + (lanes[6] + lanes[7]));
        for value in &values[blocked..] {
            sum += value;
        }
        return sum;
    }
    let mut half = len / 2;
    half -= half % PAIRWISE_BLOCK;
    pairwise_sum(&values[..half]) + pairwise_sum(&values[half..])
}

// The index of the winner. None picks the likeliest, which keeps a launch reproducible; Some(draw)
// is numpy.random.choice at a fixed uniform draw (searchsorted right in the renormalized CDF),
// which the launch goldens pin.
fn choose(weights: &[f64], draw: Option<f64>) -> usize {
    let total = pairwise_sum(weights);
    let probabilities: Vec<f64> = weights.iter().map(|weight| weight / total).collect();

    let Some(draw) = draw else {
        let mut best = 0;
        for (index, probability) in probabilities.iter().enumerate() {
            if *probability > probabilities[best] {
                best = index;
            }
        }
        return best;
    };

    let mut cdf = Vec::with_capacity(probabilities.len());
    let mut acc = 0.0;
    for probability in &probabilities {
        acc += probability;
        cdf.push(acc);
    }
    let last = cdf.last().copied().unwrap_or(1.0);
    let passed = cdf.iter().filter(|point| *point / last <= draw).count();
    passed.min(probabilities.len() - 1)
}

// The locale a territory speaks: its likeliest official language, or the one `draw` lands on
pub fn from_region(region: &str, draw: Option<f64>) -> Result<Locale, Error> {
    let territory = territories()
        .iter()
        .find(|territory| territory["type"] == region)
        .ok_or_else(|| invalid(format!("unknown territory {region:?}")))?;
    let languages = languages_of(territory);
    if languages.is_empty() {
        return Err(invalid(format!("no language data for region {region:?}")));
    }

    // Unpinned, only official languages compete: a country's most spoken language is often English
    let official: Vec<&Value> = languages
        .iter()
        .filter(|entry| draw.is_some() || entry.get("officialStatus").is_some())
        .collect();
    let candidates = if official.is_empty() {
        languages.iter().collect()
    } else {
        official
    };
    let weights: Vec<f64> = candidates
        .iter()
        .map(|entry| entry["populationPercent"].as_f64().unwrap_or_default())
        .collect();
    let language = candidates[choose(&weights, draw)]["type"]
        .as_str()
        .unwrap_or_default()
        .replace('_', "-");
    normalize_locale(&format!("{language}-{region}"))
}

// The locale a language is spoken in: the region with the most speakers, or the one `draw` lands on
pub fn from_language(language: &str, draw: Option<f64>) -> Result<Locale, Error> {
    let mut regions = Vec::new();
    let mut weights = Vec::new();
    for territory in territories() {
        let Some(entry) = languages_of(territory)
            .iter()
            .find(|entry| entry["type"] == language)
        else {
            continue;
        };
        let percent = |value: &Value| value.as_f64().unwrap_or_default();
        regions.push(territory["type"].as_str().unwrap_or_default());
        weights.push(
            percent(&entry["populationPercent"]) * percent(&territory["literacyPercent"])
                / (PERCENT_DIVISOR * PERCENT_DIVISOR)
                * percent(&territory["population"]),
        );
    }
    if regions.is_empty() {
        return Err(invalid(format!("no region data for language {language:?}")));
    }

    normalize_locale(&format!("{language}-{}", regions[choose(&weights, draw)]))
}

// A region ("CA"), a language ("de") or a tag ("de-DE", "zh-Hans-CN") as a full locale
pub fn handle_locale(locale: &str, ignore_region: bool) -> Result<Locale, Error> {
    if locale.len() > MAX_BARE_CODE_CHARS {
        return normalize_locale(locale);
    }
    if let Ok(found) = from_region(locale, None) {
        return Ok(found);
    }
    if ignore_region {
        return if language_is_known(locale) {
            Ok(Locale {
                language: locale.to_owned(),
                region: None,
                script: None,
            })
        } else {
            Err(invalid_locale(locale))
        };
    }
    from_language(locale, None).map_err(|_| invalid_locale(locale))
}

// A comma-separated locale list into the locale:* keys. The first entry sets the Intl locale;
// a longer list also sets locale:all, with the regions left out.
pub fn handle_locales(locales: &str, config: &mut Config) -> Result<(), Error> {
    let list: Vec<&str> = locales.split(',').map(str::trim).collect();

    handle_locale(list[0], false)?.write_config(config);
    if list.len() < 2 {
        return Ok(());
    }

    let mut resolved: Vec<String> = Vec::new();
    for locale in list {
        let entry = handle_locale(locale, true)?.as_string();
        if !resolved.contains(&entry) {
            resolved.push(entry);
        }
    }
    config.insert("locale:all".to_owned(), resolved.join(", ").into());
    Ok(())
}
