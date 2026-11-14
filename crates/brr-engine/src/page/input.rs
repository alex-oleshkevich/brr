use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::LazyLock;

use brr_core::Error;
use schemars::JsonSchema;
use serde::Deserialize;

const LAYOUT: &str = include_str!("../../assets/keyboard-layout.json");
const KEY_FIELD: &str = "key";
const KEYPAD_LOCATION: u32 = 3;
const SHIFT: &str = "Shift";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    None,
}

impl MouseButton {
    pub(crate) fn name(self) -> &'static str {
        match self {
            MouseButton::Left => "left",
            MouseButton::Middle => "middle",
            MouseButton::Right => "right",
            MouseButton::None => "none",
        }
    }

    pub(crate) fn mask(self) -> u32 {
        match self {
            MouseButton::Left => 1,
            MouseButton::Right => 2,
            MouseButton::Middle => 4,
            MouseButton::None => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum Modifier {
    Alt,
    Control,
    Meta,
    Shift,
}

impl Modifier {
    pub(crate) fn mask(self) -> u32 {
        match self {
            Modifier::Alt => 1,
            Modifier::Control => 2,
            Modifier::Meta => 4,
            Modifier::Shift => 8,
        }
    }

    pub(crate) fn key_name(self) -> &'static str {
        match self {
            Modifier::Alt => "Alt",
            Modifier::Control => "Control",
            Modifier::Meta => "Meta",
            Modifier::Shift => SHIFT,
        }
    }

    fn parse(name: &str) -> Option<Modifier> {
        match name {
            "Alt" => Some(Modifier::Alt),
            "Control" | "ControlOrMeta" => Some(Modifier::Control),
            "Meta" => Some(Modifier::Meta),
            "Shift" => Some(Modifier::Shift),
            _ => None,
        }
    }
}

pub fn modifiers_mask(modifiers: &[Modifier]) -> u32 {
    modifiers
        .iter()
        .fold(0, |mask, modifier| mask | modifier.mask())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClickOptions {
    pub button: MouseButton,
    pub modifiers: Vec<Modifier>,
    pub click_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitCondition {
    Selector(String),
    Text(String),
    Url(String),
    NetworkIdle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    Up,
    Move,
    Wheel,
}

// CSS pixels; `modifiers` is the CDP bitmask.
#[derive(Debug, Clone, PartialEq)]
pub struct MouseInput {
    pub kind: MouseKind,
    pub x: f64,
    pub y: f64,
    pub button: MouseButton,
    pub click_count: u32,
    pub modifiers: u32,
    pub delta_x: f64,
    pub delta_y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Down,
    Up,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInput {
    pub kind: KeyKind,
    pub key: String,
    pub code: String,
    pub key_code: u32,
    pub text: String,
    pub modifiers: u32,
}

#[derive(Clone, PartialEq, Eq)]
pub struct UploadFile {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

impl fmt::Debug for UploadFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadFile")
            .field("name", &self.name)
            .field("mime", &self.mime)
            .field("len", &self.data.len())
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Definition {
    key: String,
    key_code: u32,
    key_code_without_location: Option<u32>,
    text: Option<String>,
    location: Option<u32>,
    shift_key: Option<String>,
    shift_key_code: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyDescription {
    pub key: String,
    pub key_code: u32,
    pub key_code_without_location: u32,
    pub text: String,
    pub code: String,
    pub location: u32,
    pub shifted: Option<Box<KeyDescription>>,
}

impl KeyDescription {
    pub fn is_keypad(&self) -> bool {
        self.location == KEYPAD_LOCATION
    }
}

// Playwright's usKeyboardLayout closure: every code, every printable key and the aliases.
static KEYS: LazyLock<HashMap<String, KeyDescription>> = LazyLock::new(|| {
    let layout: BTreeMap<String, Definition> = serde_json::from_str(LAYOUT).unwrap_or_default();
    let aliases: [(&str, &[&str]); 5] = [
        ("ShiftLeft", &["Shift"]),
        ("ControlLeft", &["Control"]),
        ("AltLeft", &["Alt"]),
        ("MetaLeft", &["Meta"]),
        ("Enter", &["\n", "\r"]),
    ];
    let mut keys = HashMap::new();

    for (code, definition) in &layout {
        let mut description = KeyDescription {
            key: definition.key.clone(),
            key_code: definition.key_code,
            key_code_without_location: definition
                .key_code_without_location
                .unwrap_or(definition.key_code),
            text: definition.text.clone().unwrap_or_default(),
            code: code.clone(),
            location: definition.location.unwrap_or(0),
            shifted: None,
        };
        if definition.key.chars().count() == 1 {
            description.text.clone_from(&definition.key);
        }
        let shifted = definition.shift_key.as_ref().map(|shift_key| {
            let mut shifted = description.clone();
            shifted.key.clone_from(shift_key);
            shifted.text.clone_from(shift_key);
            if let Some(key_code) = definition.shift_key_code {
                shifted.key_code = key_code;
            }

            shifted
        });

        let mut plain = description.clone();
        plain.shifted = shifted.clone().map(Box::new);
        keys.insert(code.clone(), plain);
        for (alias_of, names) in aliases {
            if alias_of == code {
                for name in names {
                    keys.insert((*name).to_owned(), description.clone());
                }
            }
        }

        // The numpad is never picked when a character is turned into a key.
        if description.location != 0 {
            continue;
        }
        if description.key.chars().count() == 1 {
            keys.insert(description.key.clone(), description.clone());
        }
        if let Some(shifted) = shifted {
            keys.insert(shifted.key.clone(), shifted);
        }
    }

    keys
});

pub(crate) fn key_description(name: &str) -> Option<&'static KeyDescription> {
    KEYS.get(name)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyCombo {
    pub modifiers: Vec<Modifier>,
    pub key: &'static KeyDescription,
}

pub(crate) fn invalid_key(message: String) -> Error {
    Error::InvalidArgument {
        field: KEY_FIELD.to_owned(),
        message,
    }
}

// "Control+Shift+A": modifiers, then the key. A lone "+" is the plus key.
pub(crate) fn parse_combo(text: &str) -> Result<KeyCombo, Error> {
    let mut tokens: Vec<String> = Vec::new();
    let mut building = String::new();
    for ch in text.chars() {
        if ch == '+' && !building.is_empty() {
            tokens.push(std::mem::take(&mut building));
        } else {
            building.push(ch);
        }
    }
    tokens.push(building);

    let name = tokens.pop().unwrap_or_default();
    let modifiers = tokens
        .iter()
        .map(|token| {
            Modifier::parse(token)
                .ok_or_else(|| invalid_key(format!("\"{token}\" is not a modifier")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let key =
        key_description(&name).ok_or_else(|| invalid_key(format!("unknown key \"{name}\"")))?;

    Ok(KeyCombo { modifiers, key })
}

// What a key sends given the modifiers held: Shift picks the shifted form, any other modifier
// suppresses the text.
pub(crate) fn effective(description: &KeyDescription, held: &[Modifier]) -> KeyDescription {
    let shift = held.contains(&Modifier::Shift);
    let mut effective = match (&description.shifted, shift) {
        (Some(shifted), true) => (**shifted).clone(),
        _ => description.clone(),
    };
    if held.iter().any(|modifier| *modifier != Modifier::Shift) {
        effective.text.clear();
    }
    effective.shifted = None;

    effective
}
