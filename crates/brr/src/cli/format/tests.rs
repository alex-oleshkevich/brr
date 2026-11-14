use test_case::test_case;

use super::{Group, MAX_CELL_CHARS, clean, render_list, render_record, sanitize};

#[test]
fn list_aligns_names_left_and_cells_right_across_groups() {
    let groups = [
        Group::new("openai").entry("gpt-5", &["400k".to_string()]),
        Group::new("empty"),
        Group::new("local").entry("llama", &["8k".to_string()]),
    ];

    assert_eq!(
        render_list(&groups, false),
        "openai:\n    gpt-5  400k\n\nlocal:\n    llama    8k\n"
    );
}

#[test]
fn styled_list_colors_headers_and_names_only() {
    let groups = [Group::new("tabs").entry("a", &[])];

    assert_eq!(
        render_list(&groups, true),
        "\x1b[1;32mtabs:\x1b[0m\n    \x1b[1;36ma\x1b[0m\n"
    );
}

#[test]
fn control_characters_are_stripped_and_cells_capped() {
    let hostile = format!("\x1b[31mred\n{}", "x".repeat(MAX_CELL_CHARS * 2));
    let groups = [Group::new("tabs").entry("name", &[hostile])];
    let text = render_list(&groups, false);

    assert!(!text.contains('\x1b'));
    assert!(text.contains("[31mred"));
    assert_eq!(text.lines().count(), 2);
    assert!(text.trim_end().ends_with('…'));
}

#[test]
fn record_aligns_values() {
    let fields = [("id", "1".to_string()), ("version", "0.1".to_string())];

    assert_eq!(render_record(&fields), "id       1\nversion  0.1\n");
}

#[test_case("short", 5, "short" ; "at the cap")]
#[test_case("longer", 5, "longe…" ; "over the cap")]
fn clean_caps_length(text: &str, cap: usize, expected: &str) {
    assert_eq!(clean(text, cap), expected);
}

#[test]
fn multi_line_text_keeps_newlines_and_tabs_but_not_escapes() {
    let text = "a\tb\r\n\x1b[31mc\n";

    assert_eq!(sanitize(text, usize::MAX, true), "a\tb\n[31mc\n");
    assert_eq!(sanitize(text, usize::MAX, false), "ab[31mc");
}
