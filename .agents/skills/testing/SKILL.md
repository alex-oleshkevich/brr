---
name: testing
description: Testing in brr — which tier a test belongs to, where test code lives (a sibling tests.rs or tests/*.rs, never inline), keeping tests away from real config and sessions, splitting pure logic out of terminal- and I/O-coupled code, the mutation check that decides whether an assertion is load-bearing, and the manual list for what no test reaches. Use when writing or judging any test, when a change has no obvious place to be tested, and before claiming a change is verified.
---

# Testing

**Tests never share a file with the code they test.** A module's tests go in a sibling
`tests.rs` (`src/cli.rs` → `src/cli/tests.rs`) declared with `#[cfg(test)] mod tests;`; crate-level
integration tests go in `tests/*.rs`. An inline `mod tests { … }` is a defect.

## Which tier

| The code touches | Tier |
| --- | --- |
| Nothing but data (FSM tables, parsers, config, schemas, protocol framing) | plain `#[test]` |
| A service with a fake driver or in-memory store | `#[tokio::test]` driving the service API and asserting results/events |
| A real browser (CDP pipe, Juggler) | `#[tokio::test]` gated on `BRR_TEST_CHROME` / `BRR_TEST_CAMOUFOX`, skipped with a reason when unset |
| The running daemon over HTTP/CLI/MCP, the web UI | live run (`tests/live/`), reported as a manual list |

## Split pure logic out of coupled code

If a property needs a terminal, socket or process to assert, move the arithmetic or bookkeeping
out into a free function or plain struct and test that: scroll/follow math, hit-testing, URL
parsing, event application, ordering. What remains is wiring, covered by a live run or stated as
uncovered.

## Never touch the user's state

Every test and live run uses scratch `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_DATA_HOME` and
`XDG_RUNTIME_DIR`. Never read or write real sessions, trust stores or `~/.config/brr`. Never mutate
the process environment in a test (edition 2024 makes it `unsafe`, and tests run threaded); pass
values in, or use clap's `env` through `try_parse_from`.

## The mutation check

For a security- or behavior-critical decision (trust gate, input validation, Busy rejection,
event ordering), break the decision and watch the test fail:

```bash
cp src/<file>.rs /tmp/f.bak
# invert the condition, delete the guard, drop the filter
just test 2>&1 | grep -E "FAILED|panicked"
cp -f /tmp/f.bak src/<file>.rs
```

If nothing fails, the test asserts less than it appears to. That is the finding.

## What not to test

No tests of wiring, forwarding, std or clap behavior, incidental defaults, or wording. A test that
cannot fail reads as coverage and is worse than none.

## What only a human can check

A change that can only be confirmed in a real terminal ends with a numbered manual list in the
reply: what to do, what to expect, what counts as failure.

```
1. Run `brr` in an untrusted directory and press q.
   Expect: exit 0, no file written under the scratch XDG dirs.
   Fails if: a trust entry or session appears.
```

## Definition of done

- `just verify` clean: fmt-check, check, lint (`-D warnings`) and test.
- Every new decision has a test; every critical one survived the mutation check.
- Every test sits in a `tests.rs` or `tests/*.rs`.
- Anything no test reaches is in a manual list, never implied to be covered.
