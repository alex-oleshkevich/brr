use std::collections::BTreeMap;
use std::path::Path;

use brr_core::Error;
use serde_json::{Map, Value};

use super::{internal, write_atomic};

#[cfg(test)]
mod tests;

pub(super) fn merge(user_data_dir: &Path, prefs: &BTreeMap<String, Value>) -> Result<(), Error> {
    let path = user_data_dir.join("Default").join("Preferences");

    let mut root = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| internal(format!("{} is not valid JSON: {error}", path.display())))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(error) => return Err(internal(format!("{}: {error}", path.display()))),
    };

    for (key, value) in prefs {
        set(&mut root, key, value.clone());
    }

    let bytes = serde_json::to_vec(&root).map_err(internal)?;

    write_atomic(&path, &bytes)
}

fn set(root: &mut Value, dotted_key: &str, value: Value) {
    let mut node = root;

    for part in dotted_key.split('.') {
        if !node.is_object() {
            *node = Value::Object(Map::new());
        }
        if let Value::Object(map) = node {
            node = map.entry(part).or_insert(Value::Null);
        }
    }

    *node = value;
}
