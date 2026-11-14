---
name: logging
description: How brr logs with tracing. Use when adding or changing a log line, a subcommand, a retry loop or a protocol command, when deciding a log level, or when reading or testing logs.
---

# Logging

Logs answer "what did brr just do, to what, and how did it end". Every line is an event a person debugging would want; anything that repeats per frame, per key or per retry belongs at `debug` or `trace`.

## Where logs go

`brr::logging` installs one `tracing-subscriber` `fmt` subscriber in `main`, chosen per subcommand:

| Command | Sink | Why |
|---|---|---|
| `brr daemon run` | stderr (journald under systemd, file `$XDG_STATE_HOME/brr/daemon.log` when started detached) | a foreground daemon |
| `brr mcp` | `$XDG_STATE_HOME/brr/mcp.log` | stdio is the protocol |
| other CLI commands | stderr, `warn` and above only | stdout is the result |

Browser stdout/stderr never mix into daemon logs: each Instance writes `$XDG_STATE_HOME/brr/instances/<instance_id>.log`.

Daemon lines carry fields, not prose: `instance_id`, `tab_id`, `client_id`, `profile_id`, `op`.

Watch: `tail -F ~/.local/state/brr/daemon.log`.

## Levels

The default is `info`; `-q` is `warn`, `-v` `debug`, `-vv` `trace`. `RUST_LOG` replaces that default when set: `RUST_LOG=info,brr_cdp=debug`.

| Level | brr examples |
|---|---|
| `error` | can't bind; a browser failed to start; a command failed in `main` (`failed error=…`) |
| `warn` | instance crashed; proxy unhealthy; captcha failed; a Client dropped because its SSE buffer lagged |
| `info` | start/exit; one line per state transition (`instance_id`, `from`, `to`, `trigger`); one `op` line per operation with its result; build installed; sweep done |
| `debug` | admission inputs; lease acquire/release; each health check |
| `trace` | per-frame CDP/Juggler and screencast detail |

A retry loop logs the loss once at `warn`, each attempt at `debug` and the recovery at `info`.

## Writing a line

```rust
info!(peer = peer.0, request = id.0, ?command, ?result, "command");
warn!(error = format!("{error:#}"), "connection lost");
info!(%endpoint, "connected");
```

- The message is a short lowercase event (`connected`, `workspace opened`); the facts go in fields.
- **Text from outside** (paths, remote strings, client input) goes in a `?` field or as a plain `&str`/`String` field: `fmt` escapes those. A `%` field and the message text are written raw, so a newline there forges a log line. `%` is for our own types (`Endpoint`, numbers).
- Errors: `error = format!("{error:#}")`, so the whole `anyhow` chain is on one escaped line.
- `message` is reserved by `tracing`; name the field something else (`input = ?message`).
- Operations are logged with their name and ids. An input that carries content (a password, a file, page text) gets a `Debug` impl that prints its length instead, never the content.
- Add a span only when one context covers many lines; the process span is the only one today.

## Testing

Live tests (`tests/live/`) read the log the user would read: `$XDG_STATE_HOME/brr/daemon.log` in the scratch home. Assert the substrings a person searches for (`instance crashed instance_id=…`), one AC per behavior. Rust unit tests don't assert log lines.
