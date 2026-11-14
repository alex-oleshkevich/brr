# brr

A Rust daemon that installs browsers, runs them from Profiles, tracks their Tabs, reaps what sits idle
and caps load; one binary with the daemon, a CLI, MCP (HTTP + stdio) and an embedded Svelte web UI.
Read `docs/specs/2026-11-12-browser-manager-requirements.md` for what brr does,
`docs/specs/2026-11-12-brr-implementation-map.md` for where code lives, `GLOSSARY.md` for the words,
and `docs/adr/` for decisions that must not be reversed silently.

**Load the `karpathy-guidelines` and `code-style` skills before any code change, and follow the
`work-loop` skill for every task:** batch, verify, live-test, fix, at most three rounds.
Project skills live in `.agents/skills/`.

## Before writing code

Search in this order and stop at the first hit:

1. **This codebase.** A helper, type or pattern that already does it — use it the way the
   surrounding code does.
2. **std and the crates already in `Cargo.toml` / `Cargo.lock`.**
3. **A crate from the approved list below.**
4. **Only then write new code.**

**Approved crates** (2026-11-12): tokio, axum, tokio-tungstenite, futures, reqwest, rmcp, serde,
serde_json, schemars, inventory, rusqlite (bundled), rusqlite_migration, ulid, toml, toml_edit,
thiserror, anyhow, tracing, tracing-subscriber, clap, nix, libc, bytes, command-fds, object_store (aws),
rust-embed, chacha20poly1305, keyring, totp-rs, age, tar, zstd, zip, sha2, base64, rand,
dom_smoothie, htmd, adblock, aes, cbc, tokio-rustls, rustls, rustls-platform-verifier, lzma-rs, maxminddb, zeroize; dev: test-case, tempfile. Web: SvelteKit + adapter-static,
openapi-typescript, openapi-fetch, hand-written CSS, no component library; web dev: @playwright/test, @axe-core/playwright (spec §21). Extension: typescript, @types/chrome (dev).

**Any other crate or npm package needs the user's approval — hard stop.** A sub-agent that needs
one stops and reports it with its current version, why existing deps don't cover it, and the
alternative considered; it never adds it or codes around it.

## Simplicity

The user's standing instruction: **don't overengineer — simple, clean, professional code.**

- Write only the simplest solution that meets the spec. Best code is unwritten code.
- Look forward into the plan: an enum arm the next milestone needs is fine; a framework for it is not.
- No checks, guards or tests for rare edge cases the user would not see. Never cut a check that
  guards security or data (process kill, file deletion, secrets, paths from input).
- Each line justifies its existence. No helper called once, no trait with one impl, no wrapper
  that only forwards.

## Performance

brr supervises many browsers; the daemon itself must stay small and quiet.

- **Idle is free.** No polling or periodic wakeups when nothing is happening, except the named
  timers in the spec (idle, reaper, retention sweep, health checks); block on events.
- **No avoidable work** on protocol hot paths (CDP/Juggler frames, screencast, SSE fan-out): no
  re-parse or clone a borrow would serve; share large read-only data with `Arc`.
- **Bounded memory.** Every buffer, ring and queue has a named cap (§20 defaults).
- **Measure, don't guess** when changing a hot path.

## Tooling

**Use `just`, never raw `cargo`.** `just` lists recipes; `just verify` is what CI runs. A missing
or wrong recipe gets fixed in the `justfile`. A recipe body is one command; anything with a pipe,
`&&`, a loop or a branch goes in `scripts/<recipe>.sh`, which the recipe calls with `"$@"`.
Scope work to one crate while iterating: `just check-crate brr-cdp`, `just test-crate brr-cdp`.

## Hard rules

- **Do not commit, push or `bd dolt push` unless asked.**
- **Comments only for a "why" the code can't show**; no doc comments, no TODOs. `///` on clap
  types is help text and stays.
- **Separate logical code blocks with empty lines.**
- US English. Never prefix a type with `Brr` (`brr_core::Error`, not `BrrError`).
- `anyhow` in the binary; `thiserror` where a caller branches on the error.
- **CLI output goes through `brr::cli::format`** (skill `cli-output`); `print!`-family macros are banned
  by `clippy.toml`.
- **Text from pages, browsers and remote clients is hostile**: strip control sequences and cap
  length before it reaches a terminal or a log line.
- No `unwrap`/`expect` outside tests; explicit `Result` everywhere.
- Runtime state under `$XDG_RUNTIME_DIR/brr/`, data under `$XDG_DATA_HOME/brr/`, config under
  `$XDG_CONFIG_HOME/brr/`.
- Secrets never reach `Debug`, logs, events or API responses.
- Shell commands are non-interactive: `cp -f`, `mv -f`, `rm -rf`.
- No wildcard imports; import types at the top and use short names.
- Consts at the top of the file, right after imports; no inline magic numbers or strings.
- Add dependencies to the workspace `Cargo.toml`, then `workspace = true` in the crate.
- Structured logging: wide events with useful fields (`instance_id`, `tab_id`, `client_id`).
- Helpful error messages that say what to do.
- `#[derive(Copy)]` only on types with one primitive field or fieldless enums.

## Testing

- Tests live in a sibling `tests.rs` / `tests/*.rs` behind `#[cfg(test)] mod tests;`, never inline.
- Use `#[test_case]`, snake_case names; assert against shared constants, not repeated literals.
- No tautologies, no flaky sleeps; wait on events or bounded polls with a named timeout.
- Never touch real config, data or browsers' profiles: scratch `XDG_*` dirs via `tempfile`.
- Browser-dependent tests read `BRR_TEST_CHROME` (and `BRR_TEST_CAMOUFOX`) and skip with a reason
  when unset. Never assert host state.

## Task tracking

Use `bd` (beads) for all task tracking — no markdown TODO lists. Persistent knowledge goes in
`bd remember`. Run `bd prime` when context is missing.

```bash
bd ready                # find available work
bd show <id>            # view an issue
bd update <id> --claim  # claim work
bd close <id>           # complete work
```

## Finishing

1. `just fmt`, then `just verify` (fmt-check, check, lint with `-D warnings`, test) with zero warnings.
2. Delete dead code and ceremony; fix any doc the change made false.
3. File beads for remaining work and update the status of what was worked on.
4. Hand off: changed files, what was verified, bead status, and the git commands left for the user.

Feature-level plans go through the `plan-precision` skill first.
