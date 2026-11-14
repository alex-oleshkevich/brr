---
name: code-style
description: How brr code reads and where it lives — short, obvious, realistic cases only, why-comments only where the code can't say it, blank lines between blocks, one module per directory. Use whenever writing or reviewing Rust in this repo, adding a module, file, flag or config value, and when tempted to add a check, branch, helper or comment "just in case".
---

# Code style

Write code the way a careful person would: someone else reads a function once and knows what it
does, and never has to ask "what is this for?".

## Short and obvious

- **Prefer the plain version.** A `match` or an `if` beats a clever chain; two clear lines beat one
  dense line.
- **Name things so they need no comment.** A well-named local is better than a comment explaining
  an expression.
- **No ceremony.** No helper called once, no trait with one impl, no wrapper that only forwards, no
  builder for two fields, no type conversion the caller did not need.
- **Every item earns its place.** A helper, file, type or constant a stranger would have to ask
  about either shows its reason in the code or goes.
- **The owner assigns values.** No hand-numbered ids or magic numbers at call sites: the counter,
  constant or type that owns the value produces it.
- **Reuse the mechanism that already exists**: the socket is the lock, `Path::join` already
  replaces the base for absolute paths, clap's `env =` reads the environment.
- **Separate logical blocks with empty lines**: parse, validate, build, return.

## Realistic cases only

- Handle what real input and real use produce. Skip branches for paths nobody will hit: whitespace
  in a hostname, a port of zero, a file that changes between two syscalls, `dirs::home_dir()`
  returning `None`. Write the one-line fallback instead (`dirs::home_dir().unwrap_or_default()`).
- **Never cut a check that guards security or data**: input passed to `ssh` or a shell, trust
  decisions, anything that could destroy or leak user data. Those stay, however unlikely.
- When unsure whether a case is realistic, leave it out and let it fail with a clear error.

## Comments

- Names and structure carry the meaning; most code has no comments.
- **Write a `//` comment where the reason isn't in the code**: a vendor quirk, an order that
  matters, a deliberate omission, a unit or multiplier from outside. The test: would a reviewer
  ask "why?" here? Then answer it in one to three lines, with the concrete fact (`OpenAI reports
  no credits as 429`), not a restatement of the code.
- No `///` doc comments, no comments that say what the next line does, no TODOs. A shortcut with a
  known limit says so: `// shortcut: <limit>, <when to upgrade>`.
- `///` on clap types is help text and stays.

## Names

- **An id says it is an id.** A field, enum-variant field, parameter, local or closure binding
  holding an id is `<entity>_id`: `session_id: SessionId`, `message_id: MessageId`,
  `turn_id: TurnId`, `workspace_id`, `peer_id`, `request_id`, and the same for a raw `String` or
  `i64` that holds one (the `-s` value). `session` names a session, never its id.
- A bare `id` only on the entity itself: its own field and the constructor argument that sets it
  (`SessionInfo.id`, `Response::new(id, ..)`, the envelope's `Request { id }`).
- Protocol fields follow the same rule; the JSON names change with them.

## Imports and types

- For a module with many small exports, like `protocol`, import the module and qualify:
  `use crate::protocol;` and `protocol::Command`. Other imports are free.
- Paths stay paths: `PathBuf`, `&Path`, `std::env::var_os`. A `String` path breaks on non-UTF-8
  names.

## Idiomatic Rust, previews included

Every piece of Rust you show — code, plan sketches, interface previews in chat or beads — reads like
a senior Rust engineer wrote it and would compile as written. A sketch is a promise about the real
code; review it as Rust before showing it.

- **Every snippet names its file.** The first line of a code block is a comment with the path it
  belongs to, e.g. `// src/provider/mod.rs`, `# e2e/test_models.py`, `# ~/.config/brr/config.toml`.
  A usage example that spans files gets one block per file.
- **No undefined names.** Every type, function and constant a snippet uses is defined in it, brought
  in by a `use` line at its top, or fully qualified (`wasmtime::component::Instance`). A dependency
  the project doesn't have yet is named as such next to the snippet.
- **Closed sets are enums.** A fixed set of variants (API families, events, outcomes) is an `enum`
  dispatched with `match`. A trait is for an open set with several impls today.
- **No `async fn` behind `dyn`.** Async trait objects need boxing or `async-trait`; pick an enum or
  concrete types instead.
- **Streams come from tasks.** Produce async sequences from a spawned task feeding a channel; the
  consumer drops the receiver to cancel. No hand-written `Stream` impls, no `Pin<Box<dyn ..>>` in
  signatures.
- **Money and counts are integers.** Prices and costs are integer micro-units, never `f64`.
- **One type per concept.** No `Foo` and `FooInfo` describing the same thing.
- **Share, don't copy.** Large read-only data passed to many calls or tasks is `Arc`; no clone per
  call.
- **Errors by handling.** A `thiserror` enum gets a variant only when callers handle it differently;
  `anyhow` at the edges (`main`, CLI).
- **Use the standard tools.** Prefer `std`, tokio and existing dependencies over new helpers; name
  types the way the ecosystem does.

## Behavior

- **A flag works or is not accepted.** A parsed flag that changes nothing is a bug: implement it or
  refuse it with a clear error.
- **Behave like the tool the user already knows.** Paths over `ssh://` resolve the way
  `ssh host cmd` resolves them: relative to the remote home.
- **Protocol:** no version bump before 1.0; the host accepts `MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION`.

## Network, files and state (from the provider reviews, 2026-11)

- **Shared services live at their real level.** A thing every component uses (the HTTP client)
  lives at crate level (`src/http.rs`), never inside the first module that happened to need it.
- **One pooled HTTP client, connect timeout only.** A client-wide total timeout cuts off model
  streams that run for minutes; a request that needs a total limit sets `.timeout()` itself.
- **Every network body is capped.** Read through `provider::read_capped` with a named limit —
  success and error bodies alike. Never an unbounded `.text()` or `.bytes()`.
- **Environment: empty means unset**, and use the names the ecosystem already uses
  (`OPENAI_BASE_URL`, not an invented variant).
- **Order is owned, never hashed.** "The first one" comes from the owner's order (the built-in
  table), never from `HashMap` iteration.
- **Build a value through the one function that completes it.** If `Provider::model` attaches
  catalog facts, every path to a `Model` (the default included) goes through it.
- **Send the most widely accepted wire shape** (a plain string `content`) and the richer form only
  where a feature needs it (cache marks).
- **Stream decoders follow the plan's contract**: a part ends when the next one starts, every
  finish the vendor can send (refusal, `[DONE]` without a reason) maps to a finish. Hot-path calls
  fill a caller-owned buffer instead of returning a new `Vec`.
- **Work that must survive a cancel runs in its own task.** Token rotation, file writes: spawn and
  await the handle, so dropping the caller only drops the handle.
- **Never lose the only good copy.** Replace files by temp file + rename, mode set at create; refuse
  to overwrite a good cache with an empty or broken download. Shared files across tasks or
  processes follow the `locks` skill (wait, re-read, adopt or do).
- **The second copy moves to the shared parent.** When a helper, constant or test helper appears in
  a second sibling (another wire, another builder, another tests file), move it up and delete both
  copies. One constant per concept.
- **Tests assert the identifying fields** (ids, not just names) and hit every branch of a guard (a
  cap checked up front and per chunk gets a test for each).

## Code organization

- **A module is a directory** with `mod.rs` and `tests.rs` (`config/mod.rs`, `config/tests.rs`). A
  module without tests stays a single file until it gets some.
- **Group by real component, not by layer.** Crates are listed in
  `docs/specs/2026-11-12-brr-implementation-map.md`; inside brr-daemon, operations live in
  `ops/<group>.rs` next to nothing else, services in `services/<name>/`. Framing lives with what it
  frames (`brr-proto`). Each CLI subcommand's entrypoint lives under `brr/src/cli/`. `main` only
  parses, loads `Config` and dispatches.
- **One `Config`.** `main` loads it once and passes it down; defaults live on `Config`
  (`trust_file()`); code below `main` takes paths and values as arguments and never reads the
  environment.
- **Explicit arguments over globals**, so tests run in parallel on their own `Scratch` paths.

## Review question

Read the change as a stranger: is there a line you would have to stop and decode, a branch you
cannot imagine running, or an item you would ask "what is this for?" about? Simplify or delete it.
