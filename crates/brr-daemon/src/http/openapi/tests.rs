use serde_json::Value;

use super::{REF_PREFIX, openapi};
use crate::ops;

fn refs(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, inner) in object {
                match (key.as_str(), inner) {
                    ("$ref", Value::String(target)) => found.push(target.clone()),
                    _ => refs(inner, found),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| refs(item, found)),
        _ => {}
    }
}

#[test]
fn every_operation_is_described_and_refs_resolve() {
    let document = openapi();
    let text = serde_json::to_string(&document).unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(document["openapi"], "3.1.0");

    for op in ops::all() {
        let method = format!("{:?}", op.spec.http.method).to_lowercase();
        let operation = &document["paths"][op.spec.http.path][&method];

        assert_eq!(operation["operationId"], op.spec.name);
        assert_eq!(operation["x-brr-read-only"], op.spec.read_only);
        assert_eq!(operation["x-brr-mcp-tool"], Value::from(op.spec.mcp_tool));
    }

    let schemas = document["components"]["schemas"].as_object().unwrap();
    let mut targets = Vec::new();
    refs(&document, &mut targets);

    assert!(!targets.is_empty());
    for target in targets {
        let name = target.strip_prefix(REF_PREFIX).unwrap();
        assert!(schemas.contains_key(name), "dangling $ref {target}");
    }

    assert!(schemas.contains_key("Event") && schemas.contains_key("EventData"));
}

// The web UI names these types; a collision number would change whenever an operation is added.
#[test]
fn operation_input_and_output_names_carry_no_collision_number() {
    let document = openapi();
    let schemas = document["components"]["schemas"].as_object().unwrap();

    assert!(schemas.contains_key("ProfileUpdateInput"));
    assert!(schemas.contains_key("TemplateUpdateInput"));
    let numbered: Vec<_> = schemas
        .keys()
        .filter(|name| {
            let base = name.trim_end_matches(|c: char| c.is_ascii_digit());
            base.len() < name.len() && (base.ends_with("Input") || base.ends_with("Output"))
        })
        .collect();
    assert!(numbered.is_empty(), "numbered schema names: {numbered:?}");
}
