use std::path::PathBuf;

use brr_core::{Error, FILE_MARK, Method, OperationSpec};
use brr_daemon::mcp::inline_schema;
use clap::{Arg, ArgAction, ArgMatches, builder::PossibleValuesParser, value_parser};
use serde_json::{Map, Value, json};

use super::files;

#[cfg(test)]
mod tests;

pub const OUTPUT: &str = "output";
const FILE_IN: &str = "in";
const FILE_OUT: &str = "out";
const ENCODING: &str = "contentEncoding";
const PAIRS: &str = "pairs";
const KEY_VALUE: [&str; 2] = ["key", "value"];
const CLEAR_PREFIX: &str = "clear_";
const NULL: &str = "null";
const UPLOAD_DATA: &str = "data";
const UPLOAD_NAME: &str = "name";
const TRUE: &str = "true";
// `--tags a,b` and `--tags a --tags b` both give two items. Only for lists of names and ids: an
// option value or a URL may hold a comma.
const LIST_DELIMITER: char = ',';
const DELIMITED_LISTS: [&str; 2] = ["tags", "profile_ids"];
// Integer flags that take a point in time; events.list `since` is an event seq, not a time.
const EPOCH_FIELDS: [(&str, &str); 4] = [
    ("instance.list", "since"),
    ("instance.list", "until"),
    ("events.list", "from_ts"),
    ("events.list", "to_ts"),
];
const EPOCH_HELP: &str = "expects epoch milliseconds, e.g. 1794864186771";

enum Kind {
    Text,
    Integer,
    Number,
    Flag,
    Choice(Vec<String>),
    List,
    Json,
    FileIn { text: bool },
    FileList,
}

struct Field {
    name: String,
    kind: Kind,
    required: bool,
    // `--clear-<name>` sends null, so an optional field can be unset as well as set.
    nullable: bool,
    help: Option<String>,
}

pub struct OutputFile {
    pub field: String,
    pub base64: bool,
}

enum Shape {
    Fields(Vec<Field>),
    Pairs,
}

pub fn add(mut command: clap::Command, spec: &OperationSpec) -> clap::Command {
    if output_file(spec).is_some() {
        command = command.arg(
            Arg::new(OUTPUT)
                .short('o')
                .long(OUTPUT)
                .value_name("PATH")
                .value_parser(value_parser!(PathBuf))
                .help("Write the file output to PATH"),
        );
    }

    match shape(spec) {
        Shape::Pairs => command.arg(
            Arg::new(PAIRS)
                .num_args(2..)
                .value_names(KEY_VALUE)
                .required(true)
                .help("setting key and value pairs; a value is JSON, else a string"),
        ),
        Shape::Fields(fields) => {
            let file_ins = fields
                .iter()
                .filter(|field| matches!(field.kind, Kind::FileIn { .. } | Kind::FileList))
                .count();
            let positional = |name: &&str| fields.iter().find(|field| field.name == **name);

            for field in spec.cli_positional.iter().filter_map(positional) {
                command = command.arg(arg(field, true, false));
            }
            for field in fields
                .iter()
                .filter(|field| !spec.cli_positional.contains(&field.name.as_str()))
            {
                let mut flag = arg(field, false, file_ins == 1);
                if EPOCH_FIELDS.contains(&(spec.name, field.name.as_str())) {
                    flag = flag.value_parser(parse_epoch_ms);
                }
                command = command.arg(flag);
                if clearable(spec, &fields, field) {
                    command = command.arg(clear_arg(field));
                }
            }

            command
        }
    }
}

pub fn input(spec: &OperationSpec, matches: &ArgMatches) -> anyhow::Result<Value> {
    let fields = match shape(spec) {
        Shape::Pairs => return Ok(pairs_input(matches)?),
        Shape::Fields(fields) => fields,
    };

    let mut input = Map::new();
    for field in &fields {
        let cleared = clearable(spec, &fields, field) && matches.get_flag(&clear_id(&field.name));
        if cleared {
            input.insert(field.name.clone(), Value::Null);
        } else if let Some(value) = value(field, matches)? {
            input.insert(field.name.clone(), value);
        }
    }

    Ok(Value::Object(input))
}

fn shape(spec: &OperationSpec) -> Shape {
    let schema = inline_schema(spec.input_schema);

    let free_form = schema
        .get("additionalProperties")
        .is_some_and(|extra| *extra != Value::Bool(false));
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return if free_form {
            Shape::Pairs
        } else {
            Shape::Fields(Vec::new())
        };
    };

    let required = |name: &str| {
        schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|names| names.iter().any(|required| required == name))
    };

    Shape::Fields(
        properties
            .iter()
            .map(|(name, property)| {
                let kind = kind_of(property);

                Field {
                    required: required(name),
                    nullable: nullable(property),
                    help: property
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    name: name.clone(),
                    kind,
                }
            })
            .collect(),
    )
}

fn kind_of(schema: &Value) -> Kind {
    if let Some(file) = marked(schema)
        && file.get(FILE_MARK).and_then(Value::as_str) == Some(FILE_IN)
    {
        return Kind::FileIn {
            text: file.get(ENCODING).is_none(),
        };
    }

    if file_items(schema).is_some() {
        return Kind::FileList;
    }

    if let Some(choices) = choices(schema) {
        return Kind::Choice(choices);
    }

    match type_name(schema) {
        Some("string") => Kind::Text,
        Some("integer") => Kind::Integer,
        Some("number") => Kind::Number,
        Some("boolean") => Kind::Flag,
        Some("array") if schema.get("items").and_then(type_name) == Some("string") => Kind::List,
        Some(_) => Kind::Json,
        None => variants(schema)
            .iter()
            .find(|variant| type_name(variant) != Some("null"))
            .map_or(Kind::Json, kind_of),
    }
}

// An array whose items carry a file on `data` (upload_file): the CLI takes paths and builds the items.
fn file_items(schema: &Value) -> Option<&Value> {
    let data = schema.get("items")?.get("properties")?.get(UPLOAD_DATA)?;

    marked(data).filter(|file| file.get(FILE_MARK).and_then(Value::as_str) == Some(FILE_IN))
}

fn nullable(schema: &Value) -> bool {
    let in_type = match schema.get("type") {
        Some(Value::Array(names)) => names.iter().any(|name| name == NULL),
        Some(Value::String(name)) => name == NULL,
        _ => false,
    };

    in_type || variants(schema).iter().any(nullable)
}

// An Option<T> is `type: [T, "null"]` or an `anyOf` with a null variant.
fn type_name(schema: &Value) -> Option<&str> {
    match schema.get("type")? {
        Value::String(name) => Some(name),
        Value::Array(names) => names
            .iter()
            .filter_map(Value::as_str)
            .find(|name| *name != "null"),
        _ => None,
    }
}

fn variants(schema: &Value) -> &[Value] {
    ["oneOf", "anyOf"]
        .iter()
        .find_map(|key| schema.get(key).and_then(Value::as_array))
        .map_or(&[], Vec::as_slice)
}

// Enums come as `enum`, or as a `oneOf` of one `const` per documented variant.
fn choices(schema: &Value) -> Option<Vec<String>> {
    let strings = |values: &[Value]| -> Vec<String> {
        values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    };

    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return Some(strings(values));
    }

    let consts: Vec<Value> = variants(schema)
        .iter()
        .filter_map(|variant| variant.get("const").cloned())
        .collect();

    (!consts.is_empty() && consts.len() == variants(schema).len()).then(|| strings(&consts))
}

fn arg(field: &Field, positional: bool, short_i: bool) -> Arg {
    let mut arg = Arg::new(field.name.clone());

    if !positional {
        arg = arg.long(field.name.replace('_', "-"));
    }
    if let Some(help) = &field.help {
        arg = arg.help(help.clone());
    }
    if field.required {
        arg = arg.required(true);
    }

    match &field.kind {
        Kind::Text => arg,
        Kind::Integer => arg
            .value_parser(value_parser!(i64))
            .allow_negative_numbers(true),
        Kind::Number => arg
            .value_parser(value_parser!(f64))
            .allow_negative_numbers(true),
        // `--pinned` is true, `--pinned=false` is false; the `=` keeps `false` from reading as a value.
        Kind::Flag => arg
            .help(flag_help(field))
            .value_parser(value_parser!(bool))
            .num_args(0..=1)
            .require_equals(true)
            .default_missing_value(TRUE),
        Kind::FileList => arg
            .value_name("PATH")
            .value_parser(value_parser!(PathBuf))
            .num_args(1..)
            .action(ArgAction::Append),
        Kind::Choice(choices) => arg.value_parser(PossibleValuesParser::new(choices.clone())),
        Kind::List if DELIMITED_LISTS.contains(&field.name.as_str()) => arg
            .action(ArgAction::Append)
            .value_delimiter(LIST_DELIMITER),
        Kind::List => arg.action(ArgAction::Append),
        Kind::Json => arg.value_parser(parse_json),
        Kind::FileIn { .. } => {
            let arg = arg.value_name("PATH").value_parser(value_parser!(PathBuf));

            if short_i { arg.short('i') } else { arg }
        }
    }
}

fn flag_help(field: &Field) -> String {
    let what = field.help.as_deref().unwrap_or(&field.name);

    format!(
        "{what} (`--{}=false` turns it off)",
        field.name.replace('_', "-")
    )
}

// null means "unset" only where a call changes an existing thing, and only for a field that can
// hold null; a field already named `clear_<name>` (proxy update's clear_password) keeps its meaning.
fn clearable(spec: &OperationSpec, fields: &[Field], field: &Field) -> bool {
    let id = clear_id(&field.name);

    matches!(spec.http.method, Method::Patch | Method::Put)
        && field.nullable
        && !field.required
        && !fields.iter().any(|other| other.name == id)
}

fn clear_id(name: &str) -> String {
    format!("{CLEAR_PREFIX}{name}")
}

fn clear_arg(field: &Field) -> Arg {
    Arg::new(clear_id(&field.name))
        .long(format!("clear-{}", field.name.replace('_', "-")))
        .action(ArgAction::SetTrue)
        .conflicts_with(field.name.clone())
        .help(format!("Clear {}", field.name))
}

fn parse_epoch_ms(raw: &str) -> Result<i64, String> {
    raw.parse().map_err(|_| EPOCH_HELP.to_string())
}

fn parse_json(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("not valid JSON: {e}"))
}

fn value(field: &Field, matches: &ArgMatches) -> anyhow::Result<Option<Value>> {
    let name = field.name.as_str();

    Ok(match field.kind {
        Kind::FileIn { text } => {
            return matches
                .get_one::<PathBuf>(name)
                .map(|path| files::read_input(path, text))
                .transpose();
        }
        Kind::Flag => matches.get_one::<bool>(name).map(|flag| Value::Bool(*flag)),
        Kind::FileList => {
            let Some(paths) = matches.get_many::<PathBuf>(name) else {
                return Ok(None);
            };
            let items = paths
                .map(|path| {
                    let file_name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();

                    Ok(json!({
                        (UPLOAD_NAME): file_name,
                        (UPLOAD_DATA): files::read_input(path, false)?,
                    }))
                })
                .collect::<anyhow::Result<Vec<Value>>>()?;

            Some(Value::Array(items))
        }
        Kind::Text | Kind::Choice(_) => matches.get_one::<String>(name).map(|text| json!(text)),
        Kind::Integer => matches.get_one::<i64>(name).map(|number| json!(number)),
        Kind::Number => matches.get_one::<f64>(name).map(|number| json!(number)),
        Kind::List => matches
            .get_many::<String>(name)
            .map(|items| json!(items.collect::<Vec<_>>())),
        Kind::Json => matches.get_one::<Value>(name).cloned(),
    })
}

fn pairs_input(matches: &ArgMatches) -> Result<Value, Error> {
    let words: Vec<&String> = matches.get_many(PAIRS).into_iter().flatten().collect();

    if !words.len().is_multiple_of(2) {
        return Err(Error::InvalidArgument {
            field: PAIRS.to_string(),
            message: "expected <key> <value> pairs".to_string(),
        });
    }

    Ok(Value::Object(
        words
            .chunks_exact(2)
            .map(|pair| {
                let value = serde_json::from_str(pair[1]).unwrap_or_else(|_| json!(pair[1]));

                (pair[0].clone(), value)
            })
            .collect(),
    ))
}

pub fn output_file(spec: &OperationSpec) -> Option<OutputFile> {
    let schema = inline_schema(spec.output_schema);

    schema
        .get("properties")?
        .as_object()?
        .iter()
        .find_map(|(name, property)| {
            let file = marked(property)?;

            (file.get(FILE_MARK).and_then(Value::as_str) == Some(FILE_OUT)).then(|| OutputFile {
                field: name.clone(),
                base64: file.get(ENCODING).is_some(),
            })
        })
}

// The subschema carrying the file mark, looking through an Option's `anyOf`.
fn marked(schema: &Value) -> Option<&Value> {
    if schema.get(FILE_MARK).is_some() {
        return Some(schema);
    }

    variants(schema).iter().find_map(marked)
}
