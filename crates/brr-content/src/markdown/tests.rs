use super::markdown;

const ARTICLE_HTML: &str = include_str!("fixtures/article.html");
const APP_HTML: &str = include_str!("fixtures/app.html");
const URL: &str = "https://ex.com/a";

fn nested_spans(depth: usize) -> String {
    format!("{}x{}", "<span>".repeat(depth), "</span>".repeat(depth))
}

#[test]
fn article() {
    let md = markdown(ARTICLE_HTML, URL).unwrap();

    assert!(md.starts_with("# Why Small Browsers Win\n\n"), "{md}");
    assert!(
        md.contains("Running a browser for every task is wasteful."),
        "{md}"
    );
    assert!(md.contains("Throughput comes from reuse"), "{md}");
    assert!(!md.contains("Popular posts"), "{md}");
    assert!(!md.contains("Subscribe to the newsletter"), "{md}");
    assert!(md.contains("https://ex.com/b"), "{md}");
}

#[test]
fn fallback_and_deep() {
    let md = markdown(APP_HTML, URL).unwrap();

    assert!(md.contains("Add item"), "{md}");
    assert!(md.contains("Last synced 3 minutes ago."), "{md}");
    assert!(!md.contains("window.state"), "{md}");
    assert!(!md.contains(".row {"), "{md}");

    assert!(markdown(&nested_spans(10_000), URL).is_ok());
}

#[test]
fn bad_url_falls_back() {
    let md = markdown(ARTICLE_HTML, "not a url").unwrap();

    assert!(md.contains("Popular posts"), "{md}");
}

// 100 000 levels is well past what an 8 MiB stack survives unflattened.
#[test]
fn extreme_nesting_does_not_abort() {
    let md = markdown(&nested_spans(100_000), URL).unwrap();

    assert!(md.contains('x'), "{md}");
}

#[test]
fn flattening_keeps_skipped_tags_out() {
    let html = format!(
        "{}<script>leak()</script><style>.leak{{}}</style>visible{}",
        "<div>".repeat(600),
        "</div>".repeat(600)
    );
    let md = markdown(&html, URL).unwrap();

    assert!(md.contains("visible"), "{md}");
    assert!(!md.contains("leak"), "{md}");
}
