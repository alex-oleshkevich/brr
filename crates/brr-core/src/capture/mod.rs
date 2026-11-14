use std::{collections::BTreeMap, fmt, time::Duration};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer, de,
    de::{IntoDeserializer, value::StrDeserializer},
};
use serde_json::Value;

use crate::{Cookie, Error, Size, WaitUntil, model::MAX_SCREEN_SIDE};

#[cfg(test)]
mod tests;

pub const DEFAULT_SCREEN: Size = Size {
    width: 1280,
    height: 720,
};
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_QUALITY: u8 = 80;
pub const PAPER_SIZES: &[(&str, f64, f64)] = &[
    ("Letter", 8.5, 11.0),
    ("Legal", 8.5, 14.0),
    ("Tabloid", 11.0, 17.0),
    ("Ledger", 17.0, 11.0),
    ("A0", 33.1, 46.8),
    ("A1", 23.4, 33.1),
    ("A2", 16.54, 23.39),
    ("A3", 11.7, 16.54),
    ("A4", 8.27, 11.7),
    ("A5", 5.83, 8.27),
    ("A6", 4.13, 5.83),
];

pub const MAX_TIMEOUT_MS: u64 = 300_000;
const MIN_SCALE: f64 = 0.1;
const MAX_SCALE: f64 = 4.0;
const MAX_PDF_SCALE: f64 = 2.0;
const LENGTH_UNITS: [(&str, f64); 4] = [("px", 96.0), ("in", 1.0), ("cm", 2.54), ("mm", 25.4)];
const MARGIN_FIELD: &str = "margin";
const SCREENSHOT_ENDPOINT: &str = "/v1/screenshot";
const PDF_ENDPOINT: &str = "/v1/pdf";
const URL_SCHEMES: [&str; 3] = ["http", "https", "data"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenSize(pub Size);

impl Serialize for ScreenSize {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("{}x{}", self.0.width, self.0.height))
    }
}

impl<'de> Deserialize<'de> for ScreenSize {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ScreenSize, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Form {
            Text(String),
            Object(Size),
        }

        let size = match Form::deserialize(deserializer)? {
            Form::Object(size) => size,
            Form::Text(text) => {
                let (width, height) = text
                    .split_once('x')
                    .and_then(|(width, height)| Some((width.parse().ok()?, height.parse().ok()?)))
                    .ok_or_else(|| {
                        de::Error::custom(format!("`{text}` is not <width>x<height>"))
                    })?;

                Size { width, height }
            }
        };

        let sides = 1..=MAX_SCREEN_SIDE;
        if !sides.contains(&size.width) || !sides.contains(&size.height) {
            return Err(de::Error::custom(format!(
                "width and height must be within 1..={MAX_SCREEN_SIDE}"
            )));
        }

        Ok(ScreenSize(size))
    }
}

impl JsonSchema for ScreenSize {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ScreenSize".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": "^[0-9]+x[0-9]+$" })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Serialize for Area {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!(
            "{},{},{},{}",
            self.x, self.y, self.width, self.height
        ))
    }
}

impl<'de> Deserialize<'de> for Area {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Area, D::Error> {
        let text = String::deserialize(deserializer)?;
        let numbers: Vec<f64> = text
            .split(',')
            .map(|part| part.trim().parse::<f64>().ok().filter(|n| n.is_finite()))
            .collect::<Option<_>>()
            .ok_or_else(|| de::Error::custom(format!("`{text}` is not x,y,width,height")))?;

        match numbers[..] {
            [x, y, width, height] if width > 0.0 && height > 0.0 => Ok(Area {
                x,
                y,
                width,
                height,
            }),
            _ => Err(de::Error::custom(format!(
                "`{text}` is not x,y,width,height with a positive width and height"
            ))),
        }
    }
}

impl JsonSchema for Area {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Area".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": r"^\s*-?[0-9.]+\s*(,\s*-?[0-9.]+\s*){3}$"
        })
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ColorScheme {
    Dark,
    Light,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
}

impl ImageFormat {
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Webp => "image/webp",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Image,
    Font,
    Media,
    Stylesheet,
    Script,
    Xhr,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct ResourceTypes(pub Vec<ResourceType>);

impl<'de> Deserialize<'de> for ResourceTypes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ResourceTypes, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Form {
            Text(String),
            List(Vec<ResourceType>),
        }

        match Form::deserialize(deserializer)? {
            Form::List(types) => Ok(ResourceTypes(types)),
            Form::Text(text) => text
                .split(',')
                .map(|name| {
                    let name: StrDeserializer<'_, D::Error> = name.trim().into_deserializer();

                    ResourceType::deserialize(name)
                })
                .collect::<Result<_, _>>()
                .map(ResourceTypes),
        }
    }
}

impl JsonSchema for ResourceTypes {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ResourceTypes".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let list = generator.subschema_for::<Vec<ResourceType>>();

        json_schema!({ "anyOf": [list, { "type": "string" }] })
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReturnMode {
    #[default]
    Auto,
    Inline,
    Url,
    Both,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonText<T>(pub T);

impl<T: Serialize> Serialize for JsonText<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, T: de::DeserializeOwned> Deserialize<'de> for JsonText<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<JsonText<T>, D::Error> {
        let value = match Value::deserialize(deserializer)? {
            Value::String(text) => serde_json::from_str(&text)
                .map_err(|error| de::Error::custom(format!("not valid JSON text: {error}")))?,
            value => value,
        };

        serde_json::from_value(value)
            .map(JsonText)
            .map_err(de::Error::custom)
    }
}

impl<T: JsonSchema> JsonSchema for JsonText<T> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("JsonText_{}", T::schema_name()).into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let value = generator.subschema_for::<T>();

        json_schema!({ "anyOf": [value, { "type": "string" }] })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneshotKind {
    Screenshot,
    Pdf,
    Mhtml,
    Markdown,
    Content,
    Links,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct PdfOptions {
    pub paper: Option<String>,
    pub landscape: Option<bool>,
    pub margin: Option<String>,
    pub pdf_scale: Option<f64>,
    pub pages: Option<String>,
    pub print_background: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CaptureParams {
    pub url: String,
    pub screen: Option<ScreenSize>,
    pub scale: Option<f64>,
    pub device: Option<String>,
    pub theme: Option<ColorScheme>,
    pub reduced_motion: Option<bool>,
    pub locale: Option<String>,
    pub tz: Option<String>,
    pub ua: Option<String>,
    pub area: Option<Area>,
    pub selector: Option<String>,
    pub full_page: Option<bool>,
    pub wait_until: Option<WaitUntil>,
    pub wait_for: Option<String>,
    pub delay: Option<u64>,
    pub timeout: Option<u64>,
    pub hide: Option<String>,
    pub remove: Option<String>,
    pub css: Option<String>,
    pub js: Option<String>,
    pub block: Option<ResourceTypes>,
    pub format: Option<ImageFormat>,
    pub quality: Option<u8>,
    pub transparent: Option<bool>,
    pub paper: Option<String>,
    pub landscape: Option<bool>,
    #[serde(default, deserialize_with = "text_or_number")]
    pub margin: Option<String>,
    pub pdf_scale: Option<f64>,
    #[serde(default, deserialize_with = "text_or_number")]
    pub pages: Option<String>,
    pub print_background: Option<bool>,
    #[serde(default, deserialize_with = "headers_text")]
    pub headers: Option<JsonText<BTreeMap<String, String>>>,
    #[serde(default, deserialize_with = "cookies_text")]
    pub cookies: Option<JsonText<Vec<Cookie>>>,
    pub profile: Option<String>,
    pub proxy: Option<String>,
    #[serde(default)]
    pub block_ads: Option<bool>,
    #[serde(default)]
    pub dismiss_cookies: Option<bool>,
    #[serde(rename = "return")]
    pub return_mode: Option<ReturnMode>,
}

// A GET query turns the text `1` or `0` into a JSON number before it reaches these fields.
fn text_or_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Form {
        Text(String),
        Number(serde_json::Number),
    }

    Ok(
        Option::<Form>::deserialize(deserializer)?.map(|form| match form {
            Form::Text(text) => text,
            Form::Number(number) => number.to_string(),
        }),
    )
}

fn headers_text<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<JsonText<BTreeMap<String, String>>>, D::Error> {
    named_json_text("headers", deserializer)
}

fn cookies_text<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<JsonText<Vec<Cookie>>>, D::Error> {
    named_json_text("cookies", deserializer)
}

fn named_json_text<'de, D: Deserializer<'de>, T: de::DeserializeOwned>(
    field: &str,
    deserializer: D,
) -> Result<Option<JsonText<T>>, D::Error> {
    Option::deserialize(deserializer)
        .map_err(|error: D::Error| de::Error::custom(format!("`{field}`: {error}")))
}

impl CaptureParams {
    pub fn validate(&self, kind: OneshotKind) -> Result<(), Error> {
        self.check_url()?;
        self.check_kind(kind)?;
        self.check_numbers()?;
        self.check_image()?;
        self.check_pdf()
    }

    pub fn screen(&self) -> Size {
        match &self.screen {
            Some(screen) => screen.0.clone(),
            None => DEFAULT_SCREEN,
        }
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout.unwrap_or(DEFAULT_TIMEOUT_MS))
    }

    pub fn pdf(&self) -> PdfOptions {
        PdfOptions {
            paper: self.paper.clone(),
            landscape: self.landscape,
            margin: self.margin.clone(),
            pdf_scale: self.pdf_scale,
            pages: self.pages.clone(),
            print_background: self.print_background,
        }
    }

    fn check_url(&self) -> Result<(), Error> {
        let allowed = self
            .url
            .split_once(':')
            .is_some_and(|(scheme, _)| URL_SCHEMES.iter().any(|s| scheme.eq_ignore_ascii_case(s)));

        if allowed {
            return Ok(());
        }

        invalid("url", "must start with http:, https: or data:")
    }

    fn check_kind(&self, kind: OneshotKind) -> Result<(), Error> {
        let image_fields = [
            ("area", self.area.is_some()),
            ("selector", self.selector.is_some()),
            ("full_page", self.full_page.is_some()),
            ("format", self.format.is_some()),
            ("quality", self.quality.is_some()),
            ("transparent", self.transparent.is_some()),
        ];
        let pdf_fields = [
            ("paper", self.paper.is_some()),
            ("landscape", self.landscape.is_some()),
            ("margin", self.margin.is_some()),
            ("pdf_scale", self.pdf_scale.is_some()),
            ("pages", self.pages.is_some()),
            ("print_background", self.print_background.is_some()),
        ];

        if kind != OneshotKind::Screenshot
            && let Some((field, _)) = image_fields.iter().find(|(_, set)| *set)
        {
            return invalid(field, format!("only {SCREENSHOT_ENDPOINT} takes this"));
        }

        if kind != OneshotKind::Pdf
            && let Some((field, _)) = pdf_fields.iter().find(|(_, set)| *set)
        {
            return invalid(field, format!("only {PDF_ENDPOINT} takes this"));
        }

        Ok(())
    }

    fn check_numbers(&self) -> Result<(), Error> {
        if self
            .scale
            .is_some_and(|s| !(MIN_SCALE..=MAX_SCALE).contains(&s))
        {
            return invalid("scale", format!("must be within {MIN_SCALE}..={MAX_SCALE}"));
        }

        if self.timeout.is_some_and(|t| t == 0 || t > MAX_TIMEOUT_MS) {
            return invalid("timeout", format!("must be within 1..={MAX_TIMEOUT_MS} ms"));
        }

        let timeout_ms = self.timeout.unwrap_or(DEFAULT_TIMEOUT_MS);
        if self.delay.is_some_and(|d| d > timeout_ms) {
            return invalid(
                "delay",
                format!("must not exceed the timeout of {timeout_ms} ms"),
            );
        }

        Ok(())
    }

    fn check_image(&self) -> Result<(), Error> {
        validate_image(
            self.area.as_ref(),
            self.selector.is_some(),
            self.full_page,
            self.format,
            self.quality,
            self.transparent,
        )
    }

    fn check_pdf(&self) -> Result<(), Error> {
        validate_pdf(&self.pdf())
    }
}

pub fn validate_image(
    area: Option<&Area>,
    selector: bool,
    full_page: Option<bool>,
    format: Option<ImageFormat>,
    quality: Option<u8>,
    transparent: Option<bool>,
) -> Result<(), Error> {
    if area.is_some() && (selector || full_page == Some(true)) {
        return invalid("area", "cannot be combined with selector or full_page");
    }

    if let Some(quality) = quality {
        if !(1..=100).contains(&quality) {
            return invalid("quality", "must be within 1..=100");
        }

        if format.unwrap_or(ImageFormat::Png) == ImageFormat::Png {
            return invalid("quality", "applies to jpeg and webp only; set format");
        }
    }

    if transparent == Some(true) && format == Some(ImageFormat::Jpeg) {
        return invalid("transparent", "jpeg has no alpha channel; use png or webp");
    }

    Ok(())
}

pub fn validate_pdf(options: &PdfOptions) -> Result<(), Error> {
    if options
        .pdf_scale
        .is_some_and(|s| !(MIN_SCALE..=MAX_PDF_SCALE).contains(&s))
    {
        return invalid(
            "pdf_scale",
            format!("must be within {MIN_SCALE}..={MAX_PDF_SCALE}"),
        );
    }

    if let Some(paper) = &options.paper
        && !PAPER_SIZES
            .iter()
            .any(|(name, ..)| name.eq_ignore_ascii_case(paper))
    {
        let names: Vec<&str> = PAPER_SIZES.iter().map(|(name, ..)| *name).collect();

        return invalid(
            "paper",
            format!("`{paper}` is not one of {}", names.join(", ")),
        );
    }

    if let Some(margin) = &options.margin {
        margins_in(margin)?;
    }

    if options
        .pages
        .as_deref()
        .is_some_and(|pages| !is_page_ranges(pages))
    {
        return invalid("pages", "must look like 1-3,5");
    }

    Ok(())
}

pub fn css_length_in(value: &str) -> Result<f64, Error> {
    let value = value.trim();
    let with_unit = LENGTH_UNITS.iter().find_map(|(unit, per_inch)| {
        let number = value.strip_suffix(unit)?;

        Some(number.trim_end().parse::<f64>().ok()? / per_inch)
    });
    // CSS allows a bare number only for zero.
    let bare_zero = value.parse::<f64>().ok().filter(|n| *n == 0.0);

    match with_unit.or(bare_zero) {
        Some(inches) if inches.is_finite() && inches >= 0.0 => Ok(inches),
        _ => invalid(
            MARGIN_FIELD,
            format!("`{value}` is not a length such as 10mm, 1in, 2cm or 96px"),
        ),
    }
}

pub fn margins_in(value: &str) -> Result<[f64; 4], Error> {
    let lengths = value
        .split_whitespace()
        .map(css_length_in)
        .collect::<Result<Vec<_>, _>>()?;

    match lengths[..] {
        [all] => Ok([all; 4]),
        [top, right, bottom, left] => Ok([top, right, bottom, left]),
        _ => invalid(MARGIN_FIELD, "takes 1 or 4 lengths"),
    }
}

fn is_page_ranges(pages: &str) -> bool {
    let number = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());

    pages.split(',').all(|range| match range.split_once('-') {
        Some((first, last)) => number(first) && number(last),
        None => number(range),
    })
}

fn invalid<T>(field: &str, message: impl fmt::Display) -> Result<T, Error> {
    Err(Error::InvalidArgument {
        field: field.to_string(),
        message: message.to_string(),
    })
}
