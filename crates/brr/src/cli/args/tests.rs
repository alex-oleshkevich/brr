use brr_core::{Http, Method, OperationSpec, Toolset};
use clap::Command;
use schemars::{Schema, SchemaGenerator, json_schema};
use serde_json::{Value, json};

use crate::cli::{
    args::{add, input, output_file},
    command, leaf_op,
};

const TAB_ID: &str = "01JABCDEFGHJKMNPQRSTVWXYZ0";

fn parse(argv: &[&str]) -> Value {
    let matches = command().get_matches_from(argv);
    let (op, leaf) = leaf_op(&matches).expect("an op command");

    input(op.spec, leaf).expect("input")
}

#[test]
fn navigate_flags_and_positional_become_input() {
    let input = parse(&[
        "brr",
        "tab",
        "navigate",
        "https://x",
        "--tab-id",
        TAB_ID,
        "--wait-until",
        "domcontentloaded",
    ]);

    assert_eq!(
        input,
        json!({"url": "https://x", "tab_id": TAB_ID, "wait_until": "domcontentloaded"})
    );
}

#[test]
fn settings_set_takes_key_value_pairs_with_json_values() {
    let input = parse(&[
        "brr",
        "settings",
        "set",
        "ui.theme",
        "dark",
        "limits.max",
        "3",
    ]);

    assert_eq!(input, json!({"ui.theme": "dark", "limits.max": 3}));
}

#[test]
fn settings_set_rejects_a_key_without_value() {
    let matches = command().get_matches_from(["brr", "settings", "set", "a", "b", "c"]);
    let (op, leaf) = leaf_op(&matches).expect("an op command");

    assert!(input(op.spec, leaf).is_err());
}

#[test]
fn unset_flags_are_left_out() {
    assert_eq!(parse(&["brr", "profile", "list"]), json!({}));
}

#[test]
fn tags_take_commas_or_repeated_flags() {
    let input = try_parse(&[
        "brr",
        "profile",
        "create",
        "work",
        "--template",
        "chromium-headless",
        "--tags",
        "a,b",
        "--tags",
        "c",
    ])
    .expect("parses");

    assert_eq!(input["tags"], json!(["a", "b", "c"]));
}

fn import_input(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "properties": {
            "profile": {"type": "string"},
            "data": {"type": "string", "x-brr-file": "in"},
            "format": {"type": "string", "enum": ["netscape", "json"]}
        },
        "required": ["profile", "data"]
    })
}

fn export_output(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "properties": {
            "count": {"type": "integer"},
            "data": {"type": "string", "x-brr-file": "out", "contentEncoding": "base64"}
        }
    })
}

static COOKIES_IMPORT: OperationSpec = OperationSpec {
    name: "test.import",
    group: "test",
    toolset: Toolset::System,
    summary: "test",
    http: Http {
        method: Method::Post,
        path: "/test",
    },
    cli: &["import"],
    cli_positional: &["profile"],
    mcp_tool: None,
    read_only: false,
    capability: None,
    input_schema: import_input,
    output_schema: export_output,
};

#[test]
fn file_in_text() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("cookies.txt");
    std::fs::write(&file, "a\tb").expect("write");

    let command = add(Command::new("import"), &COOKIES_IMPORT);
    let matches = command.get_matches_from([
        "import",
        "work",
        "-i",
        file.to_str().expect("utf-8"),
        "--format",
        "netscape",
    ]);

    assert_eq!(
        input(&COOKIES_IMPORT, &matches).expect("input"),
        json!({"profile": "work", "data": "a\tb", "format": "netscape"})
    );
}

#[test]
fn output_flag_exists_only_for_ops_with_a_file_output() {
    let field = output_file(&COOKIES_IMPORT).expect("file output");

    assert_eq!(field.field, "data");
    assert!(field.base64);
    assert!(
        add(Command::new("import"), &COOKIES_IMPORT)
            .try_get_matches_from(["import", "work", "-i", "x", "-o", "out"])
            .is_ok()
    );
    assert!(output_file(spec_without_file_output()).is_none());
}

fn spec_without_file_output() -> &'static OperationSpec {
    brr_daemon::ops::find("settings.get").expect("op").spec
}

const DIALOG: [&str; 3] = ["brr", "tab", "handle-dialog"];

fn try_parse(argv: &[&str]) -> Result<Value, clap::Error> {
    let matches = command().try_get_matches_from(argv)?;
    let (op, leaf) = leaf_op(&matches).expect("an op command");

    Ok(input(op.spec, leaf).expect("input"))
}

#[test]
fn a_boolean_flag_is_true_alone_and_false_with_equals() {
    let with = |flag: &str| try_parse(&[DIALOG[0], DIALOG[1], DIALOG[2], flag]).expect("parses");

    assert_eq!(with("--accept"), json!({"accept": true}));
    assert_eq!(with("--accept=true"), json!({"accept": true}));
    assert_eq!(with("--accept=false"), json!({"accept": false}));
}

#[test]
fn a_required_boolean_must_be_given() {
    assert!(try_parse(&DIALOG).is_err());
}

#[test]
fn an_optional_boolean_can_be_turned_off() {
    let input = parse(&["brr", "profile", "update", "work", "--pinned=false"]);

    assert_eq!(input, json!({"profile": "work", "pinned": false}));
}

#[test]
fn clear_sends_null_for_a_nullable_field() {
    let input = parse(&[
        "brr",
        "profile",
        "update",
        "work",
        "--clear-disk-quota-bytes",
    ]);

    assert_eq!(input, json!({"profile": "work", "disk_quota_bytes": null}));
}

#[test]
fn clear_conflicts_with_setting_the_same_field() {
    let both = try_parse(&[
        "brr",
        "profile",
        "update",
        "work",
        "--disk-quota-bytes",
        "5",
        "--clear-disk-quota-bytes",
    ]);

    assert!(both.is_err());
}

#[test]
fn a_required_field_has_no_clear_flag() {
    let result = try_parse(&["brr", "profile", "update", "--clear-profile"]);

    assert!(result.is_err());
}

#[test]
fn upload_file_reads_local_paths_into_items() {
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("note.txt");
    std::fs::write(&file, "hi").expect("write");
    let path = file.to_str().expect("utf-8");

    let input = parse(&[
        "brr",
        "tab",
        "upload-file",
        "--selector",
        "input",
        "--files",
        path,
    ]);

    assert_eq!(
        input,
        json!({"selector": "input", "files": [{"name": "note.txt", "data": "aGk="}]})
    );
}

#[test]
fn clear_exists_only_on_update_operations() {
    let result = try_parse(&["brr", "tab", "navigate", "https://x", "--clear-timeout"]);

    assert!(result.is_err());
}

#[test]
fn a_field_named_clear_x_keeps_its_own_meaning() {
    let input = parse(&["brr", "proxy", "update", "p", "--clear-password"]);

    assert_eq!(input, json!({"proxy_id": "p", "clear_password": true}));
}
