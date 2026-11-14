use std::collections::BTreeMap;

use brr_core::{ErrorBody, Event, EventData, Method, OperationSpec};
use schemars::{Schema, generate::SchemaSettings};
use serde_json::{Map, Value, json};

use crate::{ops, wire::CLIENT_HEADER};

#[cfg(test)]
mod tests;

const COMPONENTS_PATH: &str = "/components/schemas/";
const REF_PREFIX: &str = "#/components/schemas/";

pub fn openapi() -> Value {
    let settings =
        SchemaSettings::draft2020_12().with(|s| s.definitions_path = COMPONENTS_PATH.into());
    let mut generator = settings.into_generator();

    let mut operations: Vec<_> = ops::all()
        .into_iter()
        .map(|op| {
            let input = (op.spec.input_schema)(&mut generator);
            let output = (op.spec.output_schema)(&mut generator);

            (op.spec, input, output)
        })
        .collect();
    let error_body = generator.subschema_for::<ErrorBody>();
    // Not referenced by any operation; the UI narrows stream events with them.
    generator.subschema_for::<Event>();
    generator.subschema_for::<EventData>();
    let mut schemas = generator.take_definitions(true);
    name_after_operations(&mut operations, &mut schemas);

    let mut paths = Map::new();
    for (spec, input, output) in &operations {
        let operation = operation(spec, input, output, &error_body, &schemas);
        let item = paths
            .entry(spec.http.path)
            .or_insert_with(|| Value::Object(Map::new()));
        item[method_name(spec.http.method)] = operation;
    }

    json!({
        "openapi": "3.1.0",
        "info": { "title": "brr", "version": env!("CARGO_PKG_VERSION") },
        "paths": paths,
        "components": { "schemas": schemas },
    })
}

// Each operation declares its own `Input` and `Output` structs, so schemars numbers the collisions
// (`UpdateInput5`) and the numbers shift whenever an operation is added. A generic Input or Output
// type that only one operation uses, or that carries a collision number, is renamed after that
// operation (`ProfileUpdateInput`).
fn name_after_operations(
    operations: &mut [(&'static OperationSpec, Schema, Schema)],
    schemas: &mut Map<String, Value>,
) {
    let target = |schema: &Schema| {
        let reference = schema.as_value().get("$ref")?.as_str()?;
        reference.strip_prefix(REF_PREFIX).map(str::to_owned)
    };
    let mut uses: BTreeMap<String, usize> = BTreeMap::new();
    for (_, input, output) in operations.iter() {
        for name in [target(input), target(output)].into_iter().flatten() {
            *uses.entry(name).or_default() += 1;
        }
    }

    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    for (spec, input, output) in operations.iter() {
        for (schema, suffix) in [(input, "Input"), (output, "Output")] {
            let Some(old) = target(schema) else { continue };
            let generic = old
                .trim_end_matches(|c: char| c.is_ascii_digit())
                .ends_with(suffix);
            let new = format!("{}{suffix}", pascal(spec.name));
            // A numbered name is a collision whoever shares it; the first operation in registry order names it.
            let numbered = old.ends_with(|c: char| c.is_ascii_digit());
            if generic && (uses[&old] == 1 || numbered) && !schemas.contains_key(&new) {
                renames.entry(old).or_insert(new);
            }
        }
    }

    for (old, new) in &renames {
        if let Some(definition) = schemas.remove(old) {
            schemas.insert(new.clone(), definition);
        }
    }
    let apply = |value: &mut Value| rename_refs(value, &renames);
    schemas.values_mut().for_each(apply);
    for (_, input, output) in operations.iter_mut() {
        for schema in [input, output] {
            let mut value = schema.as_value().clone();
            apply(&mut value);
            if let Ok(renamed) = Schema::try_from(value) {
                *schema = renamed;
            }
        }
    }
}

fn rename_refs(value: &mut Value, renames: &BTreeMap<String, String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get_mut("$ref")
                && let Some(new) = reference
                    .strip_prefix(REF_PREFIX)
                    .and_then(|old| renames.get(old))
            {
                *reference = format!("{REF_PREFIX}{new}");
            }
            map.values_mut().for_each(|v| rename_refs(v, renames));
        }
        Value::Array(items) => items.iter_mut().for_each(|v| rename_refs(v, renames)),
        _ => {}
    }
}

// "profile.bookmarks_import" -> "ProfileBookmarksImport"
fn pascal(operation: &str) -> String {
    operation
        .split(['.', '_'])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

fn operation(
    spec: &OperationSpec,
    input: &Schema,
    output: &Schema,
    error_body: &Schema,
    schemas: &Map<String, Value>,
) -> Value {
    let has_body = !matches!(spec.http.method, Method::Get | Method::Delete);
    let mut parameters: Vec<Value> = path_params(spec.http.path)
        .map(|name| json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" } }))
        .collect();

    if !has_body {
        parameters.extend(query_params(spec.http.path, input, schemas));
    }

    if !spec.read_only {
        parameters.push(json!({
            "name": CLIENT_HEADER,
            "in": "header",
            "required": false,
            "schema": { "type": "string" },
        }));
    }

    let mut operation = json!({
        "operationId": spec.name,
        "summary": spec.summary,
        "tags": [spec.group],
        "x-brr-read-only": spec.read_only,
        "x-brr-capability": spec.capability.map(|capability| capability.as_str()),
        "x-brr-mcp-tool": spec.mcp_tool,
        "x-brr-cli": (!spec.cli.is_empty()).then(|| spec.cli.join(" ")),
        "x-brr-toolset": spec.toolset.as_str(),
        "parameters": parameters,
        "responses": {
            "200": {
                "description": "ok",
                "content": { "application/json": { "schema": output } },
            },
            "default": {
                "description": "error",
                "content": { "application/json": { "schema": error_body } },
            },
        },
    });

    if has_body {
        operation["requestBody"] = json!({
            "required": true,
            "content": { "application/json": { "schema": body_schema(spec.http.path, input, schemas) } },
        });
    }

    operation
}

fn path_params(path: &str) -> impl Iterator<Item = &str> {
    path.split('/')
        .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
}

fn resolve<'a>(input: &'a Schema, schemas: &'a Map<String, Value>) -> &'a Value {
    let schema = input.as_value();
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|target| target.strip_prefix(REF_PREFIX))
        .and_then(|name| schemas.get(name))
        .unwrap_or(schema)
}

fn query_params(path: &str, input: &Schema, schemas: &Map<String, Value>) -> Vec<Value> {
    let schema = resolve(input, schemas);
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let required = schema.get("required").and_then(Value::as_array);

    properties
        .iter()
        .filter(|(name, _)| !path_params(path).any(|param| param == name.as_str()))
        .map(|(name, property)| {
            let is_required = required.is_some_and(|names| names.iter().any(|n| n == name));

            json!({ "name": name, "in": "query", "required": is_required, "schema": property })
        })
        .collect()
}

// Path parameters travel in the URL, so a client must not be asked to repeat them in the body.
fn body_schema(path: &str, input: &Schema, schemas: &Map<String, Value>) -> Value {
    let mut schema = resolve(input, schemas).clone();

    for param in path_params(path) {
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            properties.remove(param);
        }
        if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
            required.retain(|name| name != param);
        }
    }

    schema
}

fn method_name(method: Method) -> &'static str {
    match method {
        Method::Get => "get",
        Method::Post => "post",
        Method::Put => "put",
        Method::Patch => "patch",
        Method::Delete => "delete",
    }
}
