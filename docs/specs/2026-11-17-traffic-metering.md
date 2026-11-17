# Proxy traffic metering

Status: approved decisions (2026-11-17), building.

## 1. Goal

Show how many bytes each run, Profile and proxy moved, so paid residential traffic is visible.
Bytes only: no prices, no limits (user, 2026-11-17).

## 2. Discovery

| What | Where |
|---|---|
| Forwarder, connection serve paths | `crates/brr-netproxy/src/forward/mod.rs` (`serve` ~164, `handle` ~204 → `copy_bidirectional` ~253, `handle_direct` ~258 → ~289) |
| Forwarder per run, kept in a map | `crates/brr-daemon/src/services/proxies/mod.rs` (`Route` ~40, `forwarders` ~48, `for_launch`, `release`) |
| Run record | `crates/brr-core/src/model.rs` (`Instance`), `crates/brr-store/src/instance.rs` |
| Usage history op | `crates/brr-daemon/src/ops/usage.rs` (`usage.history`, `peak`) |
| Overview stats / History run summary / proxies list | `web/src/lib/overview/*`, `web/src/lib/runs/RunSummary.svelte`, `web/src/lib/settings/sections/Proxies.svelte` |

Measured: a browser has a forwarder when its Profile uses a proxy or it runs in the docker sandbox
(`for_launch`). Other browsers (no proxy, sandbox none) talk to the network directly and are not
measured (user chose "skip").

## 3. Design

- `brr_netproxy::Meter { sent: AtomicU64, received: AtomicU64 }` (bytes from the browser / to the
  browser). The forwarder wraps each client stream in a counting adapter, so long-lived tunnels
  (WebSockets, downloads) count as they go, not at close. `Forwarder::start(route, listen, meter)`.
- The daemon creates one `Arc<Meter>` per run in `for_launch` and keeps it in the run's `Route`.
- Running runs: `instance.get/list` read live values from the meter. When a run ends
  (`proxies::release`), the totals are written to the run record.
- Not measured runs: `traffic` is `null` and the UI says "Not measured".

## 4. Types

```rust
// brr-netproxy
pub struct Meter { pub sent: AtomicU64, pub received: AtomicU64 }

// brr-core model.rs
pub struct Traffic { pub sent_bytes: u64, pub received_bytes: u64 }
// Instance gains:
pub traffic: Option<Traffic>,   // None: no forwarder (not measured)

// usage.history points gain:
pub traffic_bytes: u64,          // sent + received of runs in the bucket (ended runs by end time, running runs now)
// new op usage.traffic {group: "profile" | "proxy", days: 1..=90} -> [{id, name, sent_bytes, received_bytes, runs}]
```

Migration 0008: `instance.sent_bytes INTEGER, instance.received_bytes INTEGER` (NULL = not measured).

## 5. UI

- History → run Summary: "Traffic: 12.4 MB (↑ 1.1 MB ↓ 11.3 MB)" or "Not measured".
- Browsers list: a Traffic column for running browsers (live).
- Overview: stat card "Proxy traffic (7 days)" + bars per day from `usage.history`.
- Settings → Proxies: per-proxy traffic (30 days) column from `usage.traffic group=proxy`.
- Profile detail: traffic (30 days).

## 6. Acceptance

```
AC-1  As an operator, I want each run's traffic, so that I see what agents cost on paid proxies.
      GIVEN a Profile with a proxy WHEN it loads a 1 MB file and stops THEN its run shows ≥ 1 MB received
      Verify by: daemon integration test with a local origin + forwarder; live test with the user's proxy
AC-2  GIVEN a running proxied browser WHEN I list browsers THEN its traffic grows while it downloads
      Verify by: integration test reading instance.get twice
AC-3  GIVEN a browser with no proxy and sandbox none THEN traffic shows "Not measured"
      Verify by: integration test
AC-4  GIVEN runs on two proxies WHEN usage.traffic group=proxy THEN each proxy's totals are its runs' sums
      Verify by: store/ops test
```

## 7. Struck

- ~~Price per GB, daily caps~~: user chose bytes only, no limits.
- ~~Route unproxied browsers through a forwarder~~: user chose not measured.
- ~~Per-domain breakdown~~: not asked; would need host tracking per connection.
