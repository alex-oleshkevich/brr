---
name: locks
description: How brr coordinates shared files and in-process state across tasks and processes — file locks, mutexes, wait-then-adopt, cancel-safe writes. Use when two brr processes or two tasks can touch the same file, token, cache or state, when adding a lock, try_lock or Mutex, or when reviewing refresh, login or cache code.
---

# Locks

Several brr processes run at once (a host, `brr exec`, `brr models refresh`, a second TUI), and
inside one process many tasks do. Anything they share — a token file, the model catalog, a cache —
follows one pattern.

## The pattern: wait, re-read, adopt or do

```rust
// src/provider/catalog/refresh.rs (shape; the Codex token renewal is the same)
let before = modified(&cache);
let _lock = tokio::task::spawn_blocking(move || lock::acquire(&lock_path)).await??;
if modified(&cache) != before {
    return Ok(Refresh::Adopted); // someone did the work while we waited; use theirs
}
let rows = fetch(http, url).await?;
write(&cache, &rows)?; // temp file + rename, still under the lock
```

1. **Remember what you saw** before taking the lock (file mtime, the token you hold).
2. **Wait for the lock.** Always block, never `try_lock`-and-skip: a skipper keeps stale state and
   never learns the result. Blocking happens in a background task, so nothing waits on screen.
3. **Re-read under the lock.** If the shared thing changed since step 1, another holder did the
   work: **adopt** it and stop. This is what makes N concurrent callers cost one download or one
   token refresh.
4. **Otherwise do the work and write** while still holding the lock; the lock drops with the guard.

Real instances: `catalog::refresh` (`models.lock`, adopts a newer `models.json`) and
`credentials::codex::Codex::renew` (`codex.lock`, adopts a token another process rotated).

## Which lock

| Shared between | Use | Why |
|---|---|---|
| processes | `crate::lock::acquire` on a `<name>.lock` file next to the data | the OS drops it when a process dies, so no leases, TTLs, owners or stale-lock cleanup |
| one owner, refuse the rest | `crate::lock::try_acquire`: `None` is an error to the caller, never a silent skip | a session's `messages.db` has one writing process; a second gets `InUse` |
| tasks in one process | `tokio::sync::Mutex` held across the await | the second caller waits and then sees the first one's result (re-check under the lock) |
| both | the mutex outside, the file lock inside the critical section | one task per process reaches the file lock |

- `lock::acquire` blocks the thread: inside `spawn_blocking` on async code.
- Lock a separate `.lock` file, never the data file itself: the data file is replaced by rename,
  and a lock on the old inode protects nothing.
- A socket bound to a path already is a lock (the server); don't add a second one.

## Rules

- **Check freshness under the lock, not before it.** A check before the lock races; a check after
  it is the whole point.
- **Never lose the only good copy.** Write a temp file with its final mode set at create, then
  rename over the old one. Refuse to replace good data with an empty or broken download.
- **Work that must finish survives a cancel.** If dropping the caller midway would lose state (a
  rotated refresh token not yet written), run the critical section in its own `tokio::spawn` and
  await the handle; a cancel then drops only the handle.
- **Don't hold a `std::sync::Mutex` across an `.await`.** Use `tokio::sync::Mutex` when the guard
  lives across awaits; plain data guarded for a few lines can use `std`.
- **No polling for a lock.** Block on it; never loop on `try_lock` with sleeps.

## Tests

- Two holders on one file are two `File` opens in one test process: `flock` conflicts across open
  file descriptions, so no second process is needed.
- Order a waiter deterministically without sleeps: hold the lock in the test, `tokio::spawn` the
  waiter, `tokio::task::yield_now().await` (it runs until it blocks on the lock), change the shared
  file, release, then assert the waiter adopted and made no network request.
- Mutation-check each guard: removing the lock, the re-read or the adopt branch must fail a test.
