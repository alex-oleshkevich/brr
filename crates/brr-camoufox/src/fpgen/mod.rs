pub mod model;
pub mod network;

#[cfg(test)]
mod tests;

use brr_core::Error;
use serde_json::{Map, Value};

use self::{
    model::Model,
    network::{Allowed, Evidence, Network, no_match},
};
use crate::pyrandom::PyRandom;

pub enum Constraint {
    Equals(Value),
    AnyOf(Vec<Value>),
}

// Keyed by node name or a dotted path into a node's value; keys and strings compare
// case-insensitively. Order is the caller's; a later condition on a node replaces an earlier one
pub struct Conditions(pub Vec<(String, Constraint)>);

pub fn generate(
    model: &Model,
    conditions: &Conditions,
    rng: &mut PyRandom,
) -> Result<Value, Error> {
    let network = model.network();
    let evidence = build_evidence(model, conditions)?;
    network.validate(&evidence)?;
    let sampled = network.sample(&evidence, rng)?;

    let mut fingerprint = Map::new();
    for (node, &id) in network.nodes.iter().zip(&sampled) {
        let value = parse(&model.lookup_text(&network.ids[id as usize])?)?;
        insert_path(&mut fingerprint, &node.name, value)?;
    }
    Ok(Value::Object(fingerprint))
}

fn build_evidence(model: &Model, conditions: &Conditions) -> Result<Evidence, Error> {
    let network = model.network();
    let mut evidence: Evidence = Vec::with_capacity(conditions.0.len());

    for (key, constraint) in &conditions.0 {
        let (node, nested) = resolve(network, key)?;
        let alternatives = match constraint {
            Constraint::Equals(value) => std::slice::from_ref(value),
            Constraint::AnyOf(values) if !values.is_empty() => values.as_slice(),
            Constraint::AnyOf(_) => return Err(no_match(&format!("{key} in []"))),
        };

        // The value each possible id holds at the nested path, if it has that path
        let mut candidates = Vec::with_capacity(network.nodes[node].possible.len());
        for &id in &network.nodes[node].possible {
            let value = parse(&model.lookup_text(&network.ids[id as usize])?.to_lowercase())?;
            if let Some(found) = at_path(&value, &nested) {
                candidates.push((id, found.clone()));
            }
        }

        // fpgen compares a plain node's value with ==, a path's with Python equality
        let equals = |found: &Value, wanted: &Value| {
            if nested.is_empty() {
                plain_equals(found, wanted)
            } else {
                py_equals(found, wanted)
            }
        };

        let mut allowed = Allowed::default();
        for alternative in alternatives {
            // fpgen reads the condition through its casefolded JSON text
            let wanted = parse(&alternative.to_string().to_lowercase())?;
            let matched: Vec<u32> = candidates
                .iter()
                .filter(|(_, found)| equals(found, &wanted))
                .map(|&(id, _)| id)
                .collect();
            for &id in &matched {
                allowed.insert(id);
            }

            // fpgen checks a nested path against everything allowed so far, a plain node
            // against each alternative
            let missing = if nested.is_empty() {
                matched.is_empty()
            } else {
                allowed.is_empty()
            };
            if missing {
                return Err(no_match(&format!("{key} = {alternative}")));
            }
        }

        // A second condition on the same node replaces the first, as in fpgen
        match evidence.iter_mut().find(|(existing, _)| *existing == node) {
            Some(entry) => entry.1 = allowed,
            None => evidence.push((node, allowed)),
        }
    }
    Ok(evidence)
}

// The node a key names, and the path below it: the key itself, else its longest prefix that
// is a node
fn resolve(network: &Network, key: &str) -> Result<(usize, Vec<String>), Error> {
    if let Some(node) = network.node_index(key) {
        return Ok((node, Vec::new()));
    }

    let lowered = key.to_lowercase();
    let mut prefix = lowered.as_str();
    let mut nested = Vec::new();
    while let Some((head, tail)) = prefix.rsplit_once('.') {
        nested.insert(0, tail.to_owned());
        if let Some(node) = network.node_index(head) {
            return Ok((node, nested));
        }
        prefix = head;
    }
    Err(Error::InvalidArgument {
        field: "fingerprint".to_owned(),
        message: format!("{key} is not a node of the fingerprint model"),
    })
}

fn at_path<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.as_object()?.get(key))
}

fn plain_equals(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

// Python's ==: 1 == 1.0 == True
fn py_equals(a: &Value, b: &Value) -> bool {
    if let (Some(x), Some(y)) = (py_number(a), py_number(b)) {
        return x == y;
    }

    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| py_equals(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(key, x)| y.get(key).is_some_and(|y| py_equals(x, y)))
        }
        _ => false,
    }
}

fn py_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::Bool(flag) => Some(f64::from(*flag)),
        _ => None,
    }
}

// fpgen unflattens the dotted node names into nested objects
fn insert_path(root: &mut Map<String, Value>, name: &str, value: Value) -> Result<(), Error> {
    let mut parts = name.split('.').peekable();
    let mut current = root;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            current.insert(part.to_owned(), value);
            return Ok(());
        }
        current = current
            .entry(part.to_owned())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .ok_or_else(|| Error::Internal {
                message: format!("fingerprint node {name} collides with a node above it"),
            })?;
    }
    Ok(())
}

fn parse(text: &str) -> Result<Value, Error> {
    serde_json::from_str(text).map_err(|err| Error::Internal {
        message: format!("decoding a fingerprint value: {err}"),
    })
}
