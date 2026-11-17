use std::{collections::BTreeSet, path::Path};

use base64::{Engine, engine::general_purpose::STANDARD};

use brr_core::Error;

use serde_json::{Map, Value};

use super::{
    args::OutputFile,
    files,
    format::{self, Group},
};

#[cfg(test)]
mod tests;

const NEXT_CURSOR: &str = "next_cursor";
const NAME_FIELDS: [&str; 3] = ["name", "id", "seq"];
// The id is a column only beside a name; without one it already names the row.
const ID_FIELD: &str = "id";
const CELL_FIELDS: [&str; 7] = ["id", "engine", "state", "kind", "source", "version", "url"];
const EMPTY_PREFIX: &str = "No";
const MIME_FIELD: &str = "mime";
const TEXT_MIMES: [&str; 2] = ["text/", "application/json"];

pub enum Plan<'a> {
    List {
        group: Group,
        next_cursor: Option<String>,
    },
    Record(Vec<(&'a str, String)>),
    Text(String),
}

pub enum Printed {
    Wrote { path: String, bytes: u64 },
    Text(String),
}

pub fn output(
    value: &Value,
    json: bool,
    file: Option<&OutputFile>,
    path: Option<&Path>,
) -> anyhow::Result<()> {
    if let Some(file) = file
        && let Some(printed) = file_output(value, json, file, path)?
    {
        return match printed {
            Printed::Wrote { path, bytes } => {
                format::status("Wrote", &format!("{path} ({bytes} bytes)"))?;

                if json {
                    format::data(&written(value, &file.field, &path, bytes).to_string())?;
                }

                Ok(())
            }
            Printed::Text(text) => Ok(format::data(&text)?),
        };
    }

    Ok(print(value, json)?)
}

// None: nothing file-specific to do, print the output like any other.
pub fn file_output(
    value: &Value,
    json: bool,
    file: &OutputFile,
    path: Option<&Path>,
) -> anyhow::Result<Option<Printed>> {
    let field = value.get(&file.field).unwrap_or(&Value::Null);

    if let Some(path) = path {
        let bytes = files::write_output(path, field, file.base64)?;

        return Ok(Some(Printed::Wrote {
            path: path.display().to_string(),
            bytes,
        }));
    }

    if json {
        return Ok(None);
    }

    if file.base64 {
        return match decoded_text(value, field) {
            Some(text) => Ok(Some(Printed::Text(text))),
            None => Err(Error::InvalidArgument {
                field: "output".to_string(),
                message: "binary output; pass -o <path>".to_string(),
            }
            .into()),
        };
    }

    Ok(Some(Printed::Text(text(field))))
}

// A text delivery (Markdown, HTML, links) prints like any text; only real binaries need -o.
fn decoded_text(value: &Value, field: &Value) -> Option<String> {
    let mime = value.get(MIME_FIELD)?.as_str()?;
    if !TEXT_MIMES.iter().any(|prefix| mime.starts_with(prefix)) {
        return None;
    }

    let bytes = STANDARD.decode(field.as_str()?).ok()?;

    String::from_utf8(bytes).ok()
}

// The output with its file field replaced, without copying the file's bytes.
fn written(value: &Value, field: &str, path: &str, bytes: u64) -> Value {
    let mut whole: Map<String, Value> = value
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| *key != field)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    whole.insert(
        field.to_string(),
        serde_json::json!({ "written": path, "bytes": bytes }),
    );

    Value::Object(whole)
}

fn print(value: &Value, json: bool) -> std::io::Result<()> {
    if json {
        return format::data(&value.to_string());
    }

    match plan(value) {
        Plan::List { group, next_cursor } => {
            if group.is_empty() && next_cursor.is_none() {
                let what = group.header().replace('_', " ");

                return format::data(&format!("{EMPTY_PREFIX} {what}"));
            }

            format::list(&[group])?;

            match next_cursor {
                Some(cursor) => format::status("More", &format!("pass --cursor {cursor}")),
                None => Ok(()),
            }
        }
        Plan::Record(fields) => format::record(&fields),
        Plan::Text(text) => format::data(&text),
    }
}

pub fn plan(value: &Value) -> Plan<'_> {
    let Some(object) = value.as_object() else {
        return Plan::Text(text(value));
    };

    match list_field(object) {
        Some((field, items)) => Plan::List {
            group: group(field, items),
            next_cursor: object
                .get(NEXT_CURSOR)
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        None => Plan::Record(
            object
                .iter()
                .map(|(key, value)| (key.as_str(), text(value)))
                .collect(),
        ),
    }
}

// A list output is one array field, optionally with the page cursor beside it.
fn list_field(object: &Map<String, Value>) -> Option<(&str, &[Value])> {
    let mut fields = object.iter().filter(|(key, _)| *key != NEXT_CURSOR);
    let (key, value) = fields.next()?;

    match value.as_array() {
        Some(items) if fields.next().is_none() => Some((key, items)),
        _ => None,
    }
}

fn group(field: &str, items: &[Value]) -> Group {
    let cell = |item: &Value, name: &str| {
        let named = item
            .get(NAME_FIELDS[0])
            .is_some_and(|value| !value.is_null());

        item.get(name)
            .filter(|value| !value.is_null() && (name != ID_FIELD || named))
            .map(text)
    };
    let columns: BTreeSet<usize> = items
        .iter()
        .flat_map(|item| {
            CELL_FIELDS
                .iter()
                .enumerate()
                .filter(|(_, name)| cell(item, name).is_some())
                .map(|(column, _)| column)
        })
        .collect();

    items.iter().fold(Group::new(field), |group, item| {
        let name = NAME_FIELDS
            .iter()
            .find_map(|name| item.get(name).filter(|value| !value.is_null()))
            .map_or_else(|| text(item), text);
        let cells: Vec<String> = columns
            .iter()
            .map(|column| cell(item, CELL_FIELDS[*column]).unwrap_or_default())
            .collect();

        group.entry(&name, &cells)
    })
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
