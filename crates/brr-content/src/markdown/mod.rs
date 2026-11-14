use std::mem;
use std::rc::Rc;
use std::thread;

use brr_core::Error;
use dom_smoothie::Readability;
use htmd::{HtmlToMarkdown, Node};

#[cfg(test)]
mod tests;

const MAX_BLANK_LINES: usize = 2;
// htmd walks the tree recursively, so nesting is capped; 512 levels fit easily in this stack.
const MAX_DEPTH: usize = 512;
const WORKER_STACK_BYTES: usize = 8 << 20;

// shortcut: html5ever is quadratic in nested <div> depth, so a hostile div tower parses slowly (bounded by input size); upgrade if real pages hit it. Callers run this under spawn_blocking.
pub fn markdown(html: &str, url: &str) -> Result<String, Error> {
    thread::scope(|scope| {
        thread::Builder::new()
            .stack_size(WORKER_STACK_BYTES)
            .spawn_scoped(scope, || convert(html, url))
            .map_err(internal)?
            .join()
            .unwrap_or_else(|_| Err(internal("markdown conversion panicked")))
    })
}

fn convert(html: &str, url: &str) -> Result<String, Error> {
    let converter = HtmlToMarkdown::builder()
        .skip_tags(vec!["head", "script", "style", "noscript", "template"])
        .build();
    let tree = converter.html_to_tree(html).map_err(internal)?;

    let markdown = if flatten(&tree) {
        converter.tree_to_markdown(&tree)
    } else {
        match readable_article(html, url) {
            Some((title, content)) => {
                let body = converter.convert(&content).map_err(internal)?;
                if title.is_empty() {
                    body
                } else {
                    format!("# {title}\n\n{body}")
                }
            }
            None => converter.tree_to_markdown(&tree),
        }
    };

    Ok(collapse_blank_lines(&markdown))
}

fn readable_article(html: &str, url: &str) -> Option<(String, String)> {
    let mut readability = Readability::new(html, Some(url), None).ok()?;
    if !readability.is_probably_readable() {
        return None;
    }

    let article = readability.parse().ok()?;
    if article.content.trim().is_empty() {
        return None;
    }

    Some((
        article.title.trim().to_string(),
        article.content.to_string(),
    ))
}

// Moves every node below `MAX_DEPTH` up to become a child of its ancestor at that depth.
// Returns whether anything moved.
fn flatten(root: &Rc<Node>) -> bool {
    let mut flattened = false;
    let mut stack = vec![(Rc::clone(root), 0)];

    while let Some((node, depth)) = stack.pop() {
        if depth < MAX_DEPTH {
            let children = node.children.borrow();
            stack.extend(children.iter().map(|c| (Rc::clone(c), depth + 1)));
            continue;
        }

        let mut pending: Vec<_> = node.children.borrow().iter().rev().cloned().collect();
        let mut descendants = Vec::new();
        while let Some(child) = pending.pop() {
            // Leaf-only children (script/style text) stay attached so skip_tags still drops them.
            let has_deep = child
                .children
                .borrow()
                .iter()
                .any(|g| !g.children.borrow().is_empty());
            if has_deep {
                let grandchildren = mem::take(&mut *child.children.borrow_mut());
                flattened = true;
                pending.extend(grandchildren.into_iter().rev());
            }
            child.parent.set(Some(Rc::downgrade(&node)));
            descendants.push(child);
        }
        *node.children.borrow_mut() = descendants;
    }

    flattened
}

fn internal(e: impl ToString) -> Error {
    Error::Internal {
        message: e.to_string(),
    }
}

fn collapse_blank_lines(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut blank_run = 0;

    for line in markdown.lines() {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > MAX_BLANK_LINES {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line);
        out.push('\n');
    }

    out.trim().to_string()
}
