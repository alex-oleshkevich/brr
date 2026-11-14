use super::{Bookmark, MAX_DEPTH, count, from_html, to_html};
use crate::Error;

fn link(title: &str, url: &str, add_date: Option<i64>) -> Bookmark {
    Bookmark {
        title: title.to_string(),
        url: Some(url.to_string()),
        add_date,
        children: Vec::new(),
    }
}

fn folder(title: &str, children: Vec<Bookmark>) -> Bookmark {
    Bookmark {
        title: title.to_string(),
        url: None,
        add_date: Some(1_700_000_000),
        children,
    }
}

fn nested(depth: usize) -> String {
    let mut html = String::from("<DL><p>\n");

    for level in 0..depth {
        html.push_str(&format!("<DT><H3>f{level}</H3>\n<DL><p>\n"));
    }

    html + &"</DL><p>\n".repeat(depth + 1)
}

#[test]
fn round_trips_folder_and_top_level_link() {
    let tree = vec![
        folder(
            "Work & <Play>",
            vec![
                link(
                    "Rust \"lang\"",
                    "https://rust-lang.org/?a=1&b=2",
                    Some(1_600_000_000),
                ),
                link("it's", "https://example.com/", None),
            ],
        ),
        link("Top", "https://top.example/", Some(5)),
    ];
    let html = to_html(&tree);
    let parsed = from_html(&html).unwrap();

    assert!(html.starts_with("<!DOCTYPE NETSCAPE-Bookmark-file-1>"));
    assert_eq!(parsed, tree);
    assert_eq!(count(&parsed), 3);
    assert_eq!(from_html(&to_html(&parsed)).unwrap(), tree);
}

#[test]
fn parses_browser_style_html() {
    let html = r#"<!DOCTYPE NETSCAPE-Bookmark-file-1>
<!-- comment --><TITLE>Bookmarks</TITLE>
<H1>Bookmarks</H1>
<dl><p>
<dt><h3 add_date="12" last_modified="13">News</h3>
<dd>a description
<dl><p>
<dt><a href="https://a.example/" ICON="data:x" add_date="14">A &amp;lt; B</a>
</dl><p>
<dt><a add_date="1">no href</a>
</dl><p>"#;

    assert_eq!(
        from_html(html).unwrap(),
        vec![Bookmark {
            title: "News".to_string(),
            url: None,
            add_date: Some(12),
            children: vec![link("A &lt; B", "https://a.example/", Some(14))],
        }]
    );
}

#[test]
fn empty_folder_without_a_list_is_kept() {
    let parsed =
        from_html("<DL><DT><H3>empty</H3><DT><A HREF=\"https://x.example/\">x</A></DL>").unwrap();

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0], folder_without_date("empty"));
    assert_eq!(count(&parsed), 1);
}

fn folder_without_date(title: &str) -> Bookmark {
    Bookmark {
        add_date: None,
        ..folder(title, Vec::new())
    }
}

#[test]
fn depth_limit_accepts_the_maximum_and_rejects_one_more() {
    assert_eq!(count(&from_html(&nested(MAX_DEPTH)).unwrap()), 0);
    assert!(matches!(
        from_html(&nested(MAX_DEPTH + 1)),
        Err(Error::InvalidArgument { field, .. }) if field == "data"
    ));
}

#[test]
fn stray_nested_lists_are_rejected() {
    assert!(matches!(
        from_html(&"<DL>".repeat(1000)),
        Err(Error::InvalidArgument { .. })
    ));
}

#[test]
fn truncated_input_keeps_what_was_read() {
    let parsed =
        from_html("<DL><DT><H3>f</H3><DL><DT><A HREF=\"https://a.example/\">a</A>").unwrap();

    assert_eq!(count(&parsed), 1);
    assert_eq!(parsed[0].children.len(), 1);
}
