use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Error;

#[cfg(test)]
mod tests;

const MAX_DEPTH: usize = 64;

const HEADER: &str = "<!DOCTYPE NETSCAPE-Bookmark-file-1>\n<META HTTP-EQUIV=\"Content-Type\" CONTENT=\"text/html; charset=UTF-8\">\n<TITLE>Bookmarks</TITLE>\n<H1>Bookmarks</H1>\n";
const DATA_FIELD: &str = "data";

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub title: String,
    pub url: Option<String>,
    pub add_date: Option<i64>,
    #[serde(default)]
    pub children: Vec<Bookmark>,
}

pub fn count(roots: &[Bookmark]) -> u32 {
    roots
        .iter()
        .map(|bookmark| u32::from(bookmark.url.is_some()) + count(&bookmark.children))
        .sum()
}

pub fn to_html(roots: &[Bookmark]) -> String {
    let mut out = String::from(HEADER);

    write_list(&mut out, roots, 0);

    out
}

fn write_list(out: &mut String, items: &[Bookmark], depth: usize) {
    let indent = "    ".repeat(depth);

    out.push_str(&format!("{indent}<DL><p>\n"));

    for item in items {
        let date = item
            .add_date
            .map(|date| format!(" ADD_DATE=\"{date}\""))
            .unwrap_or_default();
        let title = escape(&item.title);

        match &item.url {
            Some(url) => out.push_str(&format!(
                "{indent}    <DT><A HREF=\"{}\"{date}>{title}</A>\n",
                escape(url)
            )),
            None => {
                out.push_str(&format!("{indent}    <DT><H3{date}>{title}</H3>\n"));
                write_list(out, &item.children, depth + 1);
            }
        }
    }

    out.push_str(&format!("{indent}</DL><p>\n"));
}

struct Level {
    folder: Option<Bookmark>,
    items: Vec<Bookmark>,
}

enum Capture {
    Folder { add_date: Option<i64> },
    Link { url: String, add_date: Option<i64> },
}

pub fn from_html(html: &str) -> Result<Vec<Bookmark>, Error> {
    let mut levels = vec![Level {
        folder: None,
        items: Vec::new(),
    }];
    let mut capture: Option<(Capture, String)> = None;
    // A folder's title is read before its <DL> opens, so it waits here until the list starts.
    let mut pending: Option<Bookmark> = None;
    let mut rest = html;

    while let Some(open) = rest.find('<') {
        if let Some((_, title)) = &mut capture {
            title.push_str(&rest[..open]);
        }

        let Some(close) = rest[open..].find('>') else {
            break;
        };
        let tag = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];

        let (is_end, name, attrs) = split_tag(tag);

        match (is_end, name.as_str()) {
            (false, "h3") => {
                flush(&mut pending, &mut levels);
                capture = Some((
                    Capture::Folder {
                        add_date: attr(attrs, "add_date").and_then(|date| date.parse().ok()),
                    },
                    String::new(),
                ));
            }
            (false, "a") => {
                flush(&mut pending, &mut levels);
                capture = attr(attrs, "href").map(|url| {
                    let add_date = attr(attrs, "add_date").and_then(|date| date.parse().ok());

                    (Capture::Link { url, add_date }, String::new())
                });
            }
            (true, "h3" | "a") => match capture.take() {
                Some((Capture::Folder { add_date }, title)) => {
                    pending = Some(Bookmark {
                        title: unescape(&title),
                        url: None,
                        add_date,
                        children: Vec::new(),
                    });
                }
                Some((Capture::Link { url, add_date }, title)) => {
                    if let Some(level) = levels.last_mut() {
                        level.items.push(Bookmark {
                            title: unescape(&title),
                            url: Some(url),
                            add_date,
                            children: Vec::new(),
                        });
                    }
                }
                None => {}
            },
            (false, "dl") => {
                let folder = pending.take();

                if folder.is_none() && levels.len() > 1 {
                    return Err(invalid(
                        "a <DL> list must follow a folder heading".to_string(),
                    ));
                }

                let open_folders = levels.iter().filter(|level| level.folder.is_some()).count();

                if folder.is_some() && open_folders >= MAX_DEPTH {
                    return Err(invalid(format!(
                        "folders are nested deeper than {MAX_DEPTH} levels"
                    )));
                }

                levels.push(Level {
                    folder,
                    items: Vec::new(),
                });
            }
            (true, "dl") => {
                flush(&mut pending, &mut levels);
                close_level(&mut levels);
            }
            _ => {}
        }
    }

    flush(&mut pending, &mut levels);

    while levels.len() > 1 {
        close_level(&mut levels);
    }

    Ok(levels.pop().map(|level| level.items).unwrap_or_default())
}

// A folder with no <DL> after its heading is an empty folder.
fn flush(pending: &mut Option<Bookmark>, levels: &mut [Level]) {
    if let (Some(folder), Some(level)) = (pending.take(), levels.last_mut()) {
        level.items.push(folder);
    }
}

fn close_level(levels: &mut Vec<Level>) {
    if levels.len() < 2 {
        return;
    }

    let Some(level) = levels.pop() else {
        return;
    };
    let Some(parent) = levels.last_mut() else {
        return;
    };

    match level.folder {
        Some(mut folder) => {
            folder.children = level.items;
            parent.items.push(folder);
        }
        None => parent.items.extend(level.items),
    }
}

fn split_tag(tag: &str) -> (bool, String, &str) {
    let tag = tag.trim_start();
    let (is_end, tag) = match tag.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, tag),
    };
    let name_end = tag
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(tag.len());

    (
        is_end,
        tag[..name_end].to_ascii_lowercase(),
        &tag[name_end..],
    )
}

fn attr(attrs: &str, wanted: &str) -> Option<String> {
    let mut rest = attrs;

    loop {
        rest = rest.trim_start();

        let key_end = rest.find(|c: char| c == '=' || c.is_whitespace())?;
        let (key, after) = rest.split_at(key_end);
        let after = after.trim_start();

        let Some(value) = after.strip_prefix('=') else {
            rest = after;
            continue;
        };
        let value = value.trim_start().strip_prefix('"')?;
        let end = value.find('"')?;

        if key.eq_ignore_ascii_case(wanted) {
            return Some(unescape(&value[..end]));
        }

        rest = &value[end + 1..];
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

// `&amp;` goes last so `&amp;lt;` decodes to the text `&lt;`, not to `<`.
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn invalid(message: String) -> Error {
    Error::InvalidArgument {
        field: DATA_FIELD.to_string(),
        message,
    }
}
