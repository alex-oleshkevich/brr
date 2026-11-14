use serde_json::Value;

use crate::{coherence::Config, pycompat};

static NULL: Value = Value::Null;

// dict-style access over a config, as the Python and TypeScript twins read it
pub trait ConfigExt {
    // dict.get(key): None for a missing key
    fn value(&self, key: &str) -> &Value;
    fn truthy(&self, key: &str) -> bool;
    // JavaScript's Number() over the value, which is what the TypeScript twin compares with
    fn number(&self, key: &str) -> f64;
    fn set(&mut self, key: &str, value: Value);
}

impl ConfigExt for Config {
    fn value(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&NULL)
    }

    fn truthy(&self, key: &str) -> bool {
        pycompat::truthy(self.value(key))
    }

    fn number(&self, key: &str) -> f64 {
        number(self.value(key))
    }

    fn set(&mut self, key: &str, value: Value) {
        self.insert(key.to_owned(), value);
    }
}

// JavaScript's Number(): null is 0, text that is not a number is NaN
pub fn number(value: &Value) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(flag) => f64::from(*flag),
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::String(text) if text.trim().is_empty() => 0.0,
        Value::String(text) => text.trim().parse().unwrap_or(f64::NAN),
        Value::Array(_) | Value::Object(_) => f64::NAN,
    }
}

// A number the way Python prints an int result: integral values stay ints
pub fn number_value(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        Value::from(value as i64)
    } else {
        Value::from(value)
    }
}

// isinstance(value, int): a bool counts, a float does not
pub fn int_value(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number.as_i64(),
        _ => None,
    }
}
