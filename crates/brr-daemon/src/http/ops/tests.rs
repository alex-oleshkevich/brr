use serde_json::{Value, json};

use super::{assemble, scalar};
use crate::ops;

fn query(op: &str, pairs: &[(&str, &str)]) -> Value {
    let op = ops::find(op).unwrap();
    let query = pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();

    assemble(op, Default::default(), query, b"").unwrap()
}

#[test]
fn cursor_stays_a_string_and_limit_becomes_a_number() {
    let input = query("events.list", &[("cursor", "3"), ("limit", "1")]);

    assert_eq!(input, json!({ "cursor": "3", "limit": 1 }));
}

#[test]
fn booleans_follow_the_schema() {
    let flag = json!({ "anyOf": [{ "type": "boolean" }, { "type": "null" }] });
    let text = json!({ "type": ["string", "null"] });

    assert_eq!(scalar("true".into(), Some(&flag)), json!(true));
    assert_eq!(scalar("true".into(), Some(&text)), json!("true"));
    assert_eq!(scalar("true".into(), None), json!("true"));
}
