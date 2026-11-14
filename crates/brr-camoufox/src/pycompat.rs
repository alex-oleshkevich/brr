use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

use crate::fpgen::model::Ordered;

pub(crate) const SAFE_INTEGER_LIMIT: f64 = 9_007_199_254_740_992.0;
const DIGEST_HEX_LEN: usize = 20;
// repr(float) switches to scientific notation outside 1e-4 <= |x| < 1e16, orjson outside 1e-5..1e16
const PY_FLOAT_MIN_EXP: i32 = -4;
const ORJSON_FLOAT_MIN_EXP: i32 = -5;
const FLOAT_MAX_EXP: i32 = 16;

// Python's round(): ties go to the even neighbour
pub fn round(value: f64) -> i64 {
    value.round_ties_even() as i64
}

// Python's int(): truncates toward zero
pub fn int(value: f64) -> i64 {
    value.trunc() as i64
}

pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(members) => !members.is_empty(),
    }
}

// str() of a JSON value
pub fn str(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number_repr(number),
        Value::Array(_) | Value::Object(_) => repr(value),
    }
}

// repr() of a JSON value
pub fn repr(value: &Value) -> String {
    match value {
        Value::String(text) => str_repr(text),
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(repr).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(members) => {
            let members: Vec<String> = members
                .iter()
                .map(|(key, member)| format!("{}: {}", str_repr(key), repr(member)))
                .collect();
            format!("{{{}}}", members.join(", "))
        }
        other => str(other),
    }
}

// Unicode general category Cf (format) and Co (private use), as inclusive code point ranges
const FORMAT_AND_PRIVATE_USE: [(u32, u32); 32] = [
    (0xad, 0xad),
    (0x600, 0x605),
    (0x61c, 0x61c),
    (0x6dd, 0x6dd),
    (0x70f, 0x70f),
    (0x890, 0x891),
    (0x8e2, 0x8e2),
    (0x180e, 0x180e),
    (0x200b, 0x200f),
    (0x202a, 0x202e),
    (0x2060, 0x2064),
    (0x2066, 0x206f),
    (0xe000, 0xf8ff),
    (0xfeff, 0xfeff),
    (0xfff9, 0xfffb),
    (0x110bd, 0x110bd),
    (0x110cd, 0x110cd),
    (0x13430, 0x1343f),
    (0x1bca0, 0x1bca3),
    (0x1d173, 0x1d17a),
    (0xe0001, 0xe0001),
    (0xe0020, 0xe007f),
    (0xf0000, 0xffffd),
    (0x100000, 0x10fffd),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
];

// shortcut: unassigned code points (Cn) print raw, std has no category table; upgrade when a
// renderer string carries one
// repr() of a str
pub fn str_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for ch in text.chars() {
        let code = u32::from(ch);
        match ch {
            _ if ch == quote || ch == '\\' => {
                out.push('\\');
                out.push(ch);
            }
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ if code < 0x20 || code == 0x7f => out.push_str(&format!("\\x{code:02x}")),
            _ if ch != ' '
                && (ch.is_control() || ch.is_whitespace() || is_format_or_private(code)) =>
            {
                if code <= 0xff {
                    out.push_str(&format!("\\x{code:02x}"));
                } else if code <= 0xffff {
                    out.push_str(&format!("\\u{code:04x}"));
                } else {
                    out.push_str(&format!("\\U{code:08x}"));
                }
            }
            _ => out.push(ch),
        }
    }
    out.push(quote);
    out
}

fn is_format_or_private(code: u32) -> bool {
    FORMAT_AND_PRIVATE_USE
        .iter()
        .any(|&(first, last)| (first..=last).contains(&code))
}

fn number_repr(number: &Number) -> String {
    match number.as_f64() {
        Some(float) if number.is_f64() => float_repr(float),
        _ => number.to_string(),
    }
}

// Digits past which the exact expansion is checked for a tie
const EXACT_DIGITS: usize = 40;

// Shortest round-trip digits and the decimal exponent of a finite, non-zero float. When two
// shortest candidates are equally close Python keeps the even one, Rust the upper.
fn shortest_digits(value: f64) -> (String, i32) {
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits = mantissa.replace('.', "");

    let exact = format!("{:.EXACT_DIGITS$e}", value.abs());
    let (exact_mantissa, exact_exponent) = exact.split_once('e').unwrap_or((&exact, "0"));
    let exact_digits = exact_mantissa.replace('.', "");
    let (kept, tail) = exact_digits.split_at(digits.len().min(exact_digits.len()));
    let is_tie = tail.starts_with('5') && tail[1..].bytes().all(|digit| digit == b'0');
    let kept_is_even = kept
        .bytes()
        .last()
        .is_some_and(|digit| (digit - b'0').is_multiple_of(2));
    if is_tie && kept_is_even {
        return (kept.to_owned(), exact_exponent.parse().unwrap_or(0));
    }

    (digits, exponent.parse().unwrap_or(0))
}

fn fixed_notation(digits: &str, exp: i32) -> String {
    if exp >= 0 {
        let integral = exp as usize + 1;
        if digits.len() <= integral {
            return format!("{digits}{}.0", "0".repeat(integral - digits.len()));
        }
        return format!("{}.{}", &digits[..integral], &digits[integral..]);
    }
    format!("0.{}{digits}", "0".repeat(exp.unsigned_abs() as usize - 1))
}

fn scientific_mantissa(digits: &str) -> String {
    if digits.len() == 1 {
        digits.to_owned()
    } else {
        format!("{}.{}", &digits[..1], &digits[1..])
    }
}

// repr(float)
pub fn float_repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .to_owned();
    }

    let sign = if value < 0.0 { "-" } else { "" };
    let (digits, exp) = shortest_digits(value);
    if (PY_FLOAT_MIN_EXP..FLOAT_MAX_EXP).contains(&exp) {
        return format!("{sign}{}", fixed_notation(&digits, exp));
    }
    let exp_sign = if exp < 0 { '-' } else { '+' };
    format!(
        "{sign}{}e{exp_sign}{:02}",
        scientific_mantissa(&digits),
        exp.abs()
    )
}

// A float as orjson writes it
fn orjson_float(value: f64) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .to_owned();
    }

    let sign = if value < 0.0 { "-" } else { "" };
    let (digits, exp) = shortest_digits(value);
    if (ORJSON_FLOAT_MIN_EXP..FLOAT_MAX_EXP).contains(&exp) {
        return format!("{sign}{}", fixed_notation(&digits, exp));
    }
    let exp_sign = if exp < 0 { '-' } else { '+' };
    format!(
        "{sign}{}e{exp_sign}{}",
        scientific_mantissa(&digits),
        exp.abs()
    )
}

fn orjson_string(text: &str, out: &mut String) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            _ if u32::from(ch) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(ch))),
            _ => out.push(ch),
        }
    }
    out.push('"');
}

// orjson.dumps without OPT_SORT_KEYS: compact, keys in document order
pub fn orjson_dumps(value: &Ordered) -> String {
    let mut out = String::new();
    write_orjson(value, &mut out);
    out
}

fn write_orjson(value: &Ordered, out: &mut String) {
    match value {
        Ordered::Null => out.push_str("null"),
        Ordered::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Ordered::Number(number) => match number.as_f64() {
            Some(float) if number.is_f64() => out.push_str(&orjson_float(float)),
            _ => out.push_str(&number.to_string()),
        },
        Ordered::String(text) => orjson_string(text, out),
        Ordered::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_orjson(item, out);
            }
            out.push(']');
        }
        Ordered::Object(members) => {
            out.push('{');
            for (index, (key, member)) in members.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                orjson_string(key, out);
                out.push(':');
                write_orjson(member, out);
            }
            out.push('}');
        }
    }
}

// serde_json keeps object keys sorted, which is OPT_SORT_KEYS order
pub fn to_ordered(value: &Value) -> Ordered {
    match value {
        Value::Null => Ordered::Null,
        Value::Bool(flag) => Ordered::Bool(*flag),
        Value::Number(number) => Ordered::Number(number.clone()),
        Value::String(text) => Ordered::String(text.clone()),
        Value::Array(items) => Ordered::Array(items.iter().map(to_ordered).collect()),
        Value::Object(members) => Ordered::Object(
            members
                .iter()
                .map(|(key, member)| (key.clone(), to_ordered(member)))
                .collect(),
        ),
    }
}

// What the goldens hash: integral floats below 2**53 become ints, ints from 2**53 up become
// floats, as JavaScript cannot tell them apart
pub fn canon(value: &Value) -> Value {
    match value {
        Value::Number(number) => Value::Number(canon_number(number)),
        Value::Array(items) => Value::Array(items.iter().map(canon).collect()),
        Value::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, member)| (key.clone(), canon(member)))
                .collect::<Map<String, Value>>(),
        ),
        other => other.clone(),
    }
}

fn canon_number(number: &Number) -> Number {
    if let Some(float) = number.as_f64().filter(|_| number.is_f64()) {
        if float.fract() == 0.0 && float.abs() < SAFE_INTEGER_LIMIT {
            return Number::from(float as i64);
        }
        return number.clone();
    }
    match number.as_f64() {
        Some(float) if float.abs() >= SAFE_INTEGER_LIMIT => {
            Number::from_f64(float).unwrap_or_else(|| number.clone())
        }
        _ => number.clone(),
    }
}

// The first 20 hex digits of the sha256 of orjson.dumps(canon(value), OPT_SORT_KEYS)
pub fn canon_digest(value: &Value) -> String {
    digest(&to_ordered(&canon(value)))
}

// The first 20 hex digits of the sha256 of the exact orjson bytes
pub fn digest(value: &Ordered) -> String {
    let hash = Sha256::digest(orjson_dumps(value).as_bytes());
    let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    hex[..DIGEST_HEX_LEN].to_owned()
}
