use std::io::{self, IsTerminal, Write};

use clap::builder::{Styles, styling::AnsiColor};

#[cfg(test)]
mod tests;

pub const MAX_CELL_CHARS: usize = 200;

const MAX_LINE_CHARS: usize = 2000;
const NO_LIMIT: usize = usize::MAX;
const ELLIPSIS: char = '…';
const ENTRY_INDENT: &str = "    ";
const COLUMN_GAP: &str = "  ";
const VERB_WIDTH: usize = 12;
const GREEN: &str = "\x1b[1;32m";
const CYAN: &str = "\x1b[1;36m";
const YELLOW: &str = "\x1b[1;33m";
const RED: &str = "\x1b[1;31m";
const RESET: &str = "\x1b[0m";

pub const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().bold())
    .usage(AnsiColor::Green.on_default().bold())
    .literal(AnsiColor::Cyan.on_default().bold())
    .placeholder(AnsiColor::Cyan.on_default());

pub struct Group {
    header: String,
    entries: Vec<(String, Vec<String>)>,
}

impl Group {
    pub fn new(header: &str) -> Group {
        Group {
            header: header.to_string(),
            entries: Vec::new(),
        }
    }

    pub fn entry(mut self, name: &str, cells: &[String]) -> Group {
        let cells = cells
            .iter()
            .map(|cell| clean(cell, MAX_CELL_CHARS))
            .collect();
        self.entries.push((clean(name, MAX_CELL_CHARS), cells));

        self
    }
}

pub fn list(groups: &[Group]) -> io::Result<()> {
    write_stdout(&render_list(groups, io::stdout().is_terminal()))
}

pub fn data(text: &str) -> io::Result<()> {
    let text = sanitize(text, NO_LIMIT, true);
    let newline = if text.ends_with('\n') { "" } else { "\n" };

    write_stdout(&format!("{text}{newline}"))
}

pub fn record(fields: &[(&str, String)]) -> io::Result<()> {
    write_stdout(&render_record(fields))
}

pub fn status(verb: &str, text: &str) -> io::Result<()> {
    let styled = io::stderr().is_terminal();
    let verb = clean(verb, VERB_WIDTH);
    let indent = " ".repeat(VERB_WIDTH.saturating_sub(verb.chars().count()));
    let line = format!(
        "{indent}{} {}\n",
        paint(styled, GREEN, &verb),
        clean(text, MAX_LINE_CHARS)
    );

    write_stderr(&line)
}

pub fn error(text: &str, help: &[&str]) -> io::Result<()> {
    let styled = io::stderr().is_terminal();
    let mut message = format!(
        "{} {}\n",
        paint(styled, RED, "error:"),
        clean(text, MAX_LINE_CHARS)
    );

    if !help.is_empty() {
        message.push('\n');
    }
    for line in help {
        message.push_str(&format!(
            "{} {}\n",
            paint(styled, CYAN, "help:"),
            clean(line, MAX_LINE_CHARS)
        ));
    }

    write_stderr(&message)
}

pub fn warning(text: &str) -> io::Result<()> {
    let styled = io::stderr().is_terminal();

    write_stderr(&format!(
        "{} {}\n",
        paint(styled, YELLOW, "warning:"),
        clean(text, MAX_LINE_CHARS)
    ))
}

pub(crate) fn render_list(groups: &[Group], styled: bool) -> String {
    let groups: Vec<&Group> = groups
        .iter()
        .filter(|group| !group.entries.is_empty())
        .collect();
    let entries = || groups.iter().flat_map(|group| &group.entries);

    let name_width = entries()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(0);
    let columns = entries().map(|(_, cells)| cells.len()).max().unwrap_or(0);
    let cell_width = |column: usize| {
        entries()
            .filter_map(|(_, cells)| cells.get(column))
            .map(|cell| cell.chars().count())
            .max()
            .unwrap_or(0)
    };
    let cell_widths: Vec<usize> = (0..columns).map(cell_width).collect();

    let mut out = String::new();
    for (index, group) in groups.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "{}\n",
            paint(
                styled,
                GREEN,
                &format!("{}:", clean(&group.header, MAX_CELL_CHARS))
            )
        ));

        for (name, cells) in &group.entries {
            let mut line = format!(
                "{ENTRY_INDENT}{}{}",
                paint(styled, CYAN, name),
                " ".repeat(name_width - name.chars().count())
            );
            for (cell, width) in cells.iter().zip(&cell_widths) {
                line.push_str(COLUMN_GAP);
                line.push_str(&" ".repeat(width - cell.chars().count()));
                line.push_str(cell);
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }

    out
}

fn render_record(fields: &[(&str, String)]) -> String {
    let fields: Vec<(String, String)> = fields
        .iter()
        .map(|(key, value)| (clean(key, MAX_CELL_CHARS), clean(value, MAX_CELL_CHARS)))
        .collect();
    let key_width = fields
        .iter()
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0);

    fields
        .iter()
        .map(|(key, value)| {
            let padding = " ".repeat(key_width - key.chars().count());

            format!("{key}{padding}{COLUMN_GAP}{value}\n")
        })
        .collect()
}

fn clean(text: &str, cap: usize) -> String {
    sanitize(text, cap, false)
}

// Text from pages and remote clients reaches the terminal only through here. Multi-line output
// (a cookie export) keeps its newlines and tabs; everything else, escape sequences included, goes.
fn sanitize(text: &str, cap: usize, keep_layout: bool) -> String {
    let allowed = |c: &char| !c.is_control() || (keep_layout && matches!(c, '\n' | '\t'));
    let mut cleaned = String::new();

    for (kept, c) in text.chars().filter(allowed).enumerate() {
        if kept == cap {
            cleaned.push(ELLIPSIS);
            break;
        }
        cleaned.push(c);
    }

    cleaned
}

fn paint(styled: bool, color: &str, text: &str) -> String {
    if styled {
        format!("{color}{text}{RESET}")
    } else {
        text.to_string()
    }
}

fn write_stdout(text: &str) -> io::Result<()> {
    io::stdout().lock().write_all(text.as_bytes())
}

fn write_stderr(text: &str) -> io::Result<()> {
    io::stderr().lock().write_all(text.as_bytes())
}
