---
name: cli-output
description: How brr commands print. Use when a command writes anything to stdout or stderr, when adding a subcommand or flag that produces output, or when reporting an error, a warning or progress to the user.
---

# CLI output

Every byte a command prints goes through `src/cli/format`. It looks like cargo: bold green headers
and status verbs, bold cyan names the user can type back, plain text for everything else.
`clippy.toml` bans `print!`, `println!`, `eprint!` and `eprintln!`; there is no way around it.

## API

| Function | Stream | Prints |
|---|---|---|
| `list(&[Group])` | stdout | `header:` per group, entries indented 4; names and every cell aligned across all groups (names left, cells right); empty groups left out, one blank line between groups |
| `Group::new(header)`, `.entry(name, &cells)` | — | builds a group; one cell per column, never pre-padded |
| `data(text)` | stdout | one plain line: an id, a path, a JSON object |
| `status(verb, text)` | stderr | `     Updated 2 providers …`, verb right-aligned to 12 |
| `error(text, help)` | stderr | `error: text`, blank line, `help: …` lines; only `main::report` calls it |
| `warning(text)` | stderr | `warning: text`; for a problem the command works around, e.g. one provider that can't be reached |
| `count(n, noun)` | — | `1 model`, `2 models` |
| `STYLES` | — | the same palette for clap's `--help` |

Add a function here when a command needs a new element; never format at the call site.

## Rules

- **stdout is data, stderr is everything else.** Progress and summaries are `status`, so
  `brr x > file` and `brr x --json | jq` stay clean.
- **`--json`** prints one compact object per line through `data`, nothing else on stdout.
- **Errors are returned, never printed.** A command returns `Err`; `main::report` prints it and
  picks the exit code. An error that needs `help:` lines or a code other than 1 is a typed
  `thiserror` error that `report` recognises.
- **Wording:** lowercase after `error:`, no trailing period, names the user typed in backticks
  (`` unknown provider `nope` ``), the fix as `help:` not appended to the message.
- **Hostile text** (model, tool, remote, catalog) needs nothing extra: every function strips
  control characters and caps length.
- **Domain formatting** (token counts, prices, dates) lives in the command that owns it; shared
  formatting (pluralizing, layout) lives in `format`.

## Example

```text
$ brr models list --match gpt-5
openai:
    gpt-5        400k  $1.25 in /  10.00 out
    gpt-5-pro    400k $15.00 in / 120.00 out
$ brr models refresh
     Updated 235 providers and 8940 models in ~/.cache/brr/models.json
$ brr models list --provider nope
error: unknown provider `nope`; configured: openai, openrouter
```

## Common mistakes

| Mistake | Fix |
|---|---|
| `println!` / `eprintln!` / `print!` | the matching `format` function |
| A summary or "done" line on stdout | `status` on stderr |
| Padding columns, joining values into one string, or indenting by hand | one cell per value in `Group::entry`; `list` aligns |
| A helper like a pluralizer in a command file | move it into `format` |
| `Error: …`, a capitalized message, a hint inside the message | return the error; `help:` lines |
| Colour or glyphs to decorate | none; colour comes only from `format` |
| Testing output by string-matching ANSI codes | e2e asserts the plain text (pipes are never styled) |
