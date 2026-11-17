use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use test_case::test_case;

use super::{Plan, Printed, file_output, plan, written};
use crate::cli::args::OutputFile;
use crate::cli::format::render_list;

const MARKDOWN: &str = "# Example Domain\n";

#[test]
fn list_output_prints_header_name_and_cells() {
    let value = json!({
        "profiles": [{"id": "1", "name": "work", "kind": "persistent"}],
        "next_cursor": null
    });
    let Plan::List { group, next_cursor } = plan(&value) else {
        panic!("expected a list");
    };
    let text = render_list(&[group], false);

    assert_eq!(next_cursor, None);
    assert!(text.starts_with("profiles:\n"));
    let line = text
        .lines()
        .find(|line| line.contains("work"))
        .expect("work line");
    assert!(line.contains("persistent"));
}

#[test]
fn hostile_names_lose_their_escape_sequences() {
    let value = json!({"tabs": [{"id": "1", "title": "x", "url": "\u{1b}[31mhttp://x"}]});
    let Plan::List { group, .. } = plan(&value) else {
        panic!("expected a list");
    };

    assert!(!render_list(&[group], false).contains('\u{1b}'));
}

#[test]
fn other_objects_become_records_with_nested_values_as_json() {
    let value = json!({"version": "0.1", "limits": {"max": 3}, "tabs": [1], "extra": [2]});
    let Plan::Record(fields) = plan(&value) else {
        panic!("expected a record");
    };

    assert!(fields.contains(&("limits", "{\"max\":3}".to_string())));
    assert!(fields.contains(&("version", "0.1".to_string())));
}

#[test]
fn next_cursor_is_kept_for_the_hint() {
    let value = json!({"events": [], "next_cursor": "abc"});
    let Plan::List { next_cursor, .. } = plan(&value) else {
        panic!("expected a list");
    };

    assert_eq!(next_cursor.as_deref(), Some("abc"));
}

fn data_field(base64: bool) -> OutputFile {
    OutputFile {
        field: "data".to_string(),
        base64,
    }
}

#[test]
fn file_out() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("b.html");
    let value = json!({"count": 1, "data": "<html>\n<p>"});

    let printed = file_output(&value, false, &data_field(false), Some(&path))
        .expect("written")
        .expect("printed");
    let Printed::Wrote {
        bytes, path: shown, ..
    } = printed
    else {
        panic!("expected a write");
    };

    assert_eq!(std::fs::read_to_string(&path).expect("read"), "<html>\n<p>");
    assert_eq!(bytes, 10);
    let whole = written(&value, "data", &shown, bytes);
    assert_eq!(whole["data"]["bytes"], 10);
    assert_eq!(whole["count"], 1);

    let Some(Printed::Text(text)) =
        file_output(&value, false, &data_field(false), None).expect("text")
    else {
        panic!("expected text");
    };
    assert_eq!(text, "<html>\n<p>");
}

#[test_case("text/markdown", Some(MARKDOWN) ; "text prints")]
#[test_case("application/json", Some(MARKDOWN) ; "json prints")]
#[test_case("image/png", None ; "binary needs a path")]
fn base64_output_without_a_path(mime: &str, expected: Option<&str>) {
    let value = json!({"mime": mime, "data": STANDARD.encode(MARKDOWN)});

    let printed = file_output(&value, false, &data_field(true), None);

    match (printed, expected) {
        (Ok(Some(Printed::Text(text))), Some(expected)) => assert_eq!(text, expected),
        (Err(error), None) => assert!(error.to_string().contains("pass -o")),
        _ => panic!("unexpected output for {mime}"),
    }
    assert!(matches!(
        file_output(&value, true, &data_field(true), None),
        Ok(None)
    ));
}
