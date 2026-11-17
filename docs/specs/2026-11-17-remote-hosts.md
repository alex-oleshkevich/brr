# Remote Hosts: run browsers on other machines

Status: approved decisions (2026-11-17), building. Decision record: [ADR 0006](../adr/0006-remote-hosts.md).

## 1. Goal

brr stays one daemon (one UI, one database, one API). Other machines become **Hosts** that only
run browser containers. brr reaches each Host's Docker over SSH and places each launch on the Host
with room and the most free memory. Remote browsers behave like local docker-sandboxed ones: same
tools, same network rules, same proxies and metering.

## 2. Decisions (user, 2026-11-17)

| Topic | Decision |
|---|---|
| Profile data | Copied to the Host before launch and back after stop; a persistent Profile still runs once at a time. |
| Network | No network of their own; all traffic goes through brr's forwarder, tunneled over the container's stdio. |
| Placement | Automatic: the Host with room (per-Host max browsers) and the most free memory. |
| Requirement | Hosts need `sandbox.mode = docker` (remote browsers are always containers). |

## 3. Discovery

### 3.1 Local code

| What | Where |
|---|---|
| Docker run command, mounts, runner spec file | `crates/brr-engine/src/sandbox/mod.rs` (`command`, `mounts`, `copy_extensions`, `remove_container`, `sweep`, `run_docker`) |
| Runner (in container): Xvfb, proxy relay to `proxy.sock`, fd 3/4 relay, graceful stop | `crates/brr/src/cli/runner/mod.rs` |
| stdio transport into the CDP/Juggler mux | `crates/brr-proto/src/pipe.rs` (`spawn_stdio`) |
| Docker target per launch, discovery, rss via docker stats | `crates/brr-daemon/src/services/sandbox/mod.rs` |
| Launch spec, run dir, forwarder on run dir socket | `crates/brr-daemon/src/services/instances/mod.rs` (`prepare`/LaunchSpec), `services/proxies/mod.rs` (`for_launch`) |
| Admission and free memory | `crates/brr-daemon/src/services/admission/mod.rs` (`admit`, `free_ram_mb` from `host::memory()`) |
| Downloads written to the download dir, then turned into Artifacts | `crates/brr-daemon/src/services/downloads/mod.rs` |
| Settings dialog sections | `web/src/lib/settings/sections.ts` |

### 3.2 Remote facts

| Fact | Status |
|---|---|
| `docker -H ssh://ssh.aresa.me version` answers (Docker 29.1.5) in 1.4 s | measured 2026-11-17 |
| `printf … \| docker -H ssh://ssh.aresa.me run --rm -i --network none --entrypoint cat <brr image>` echoes stdin to stdout | measured: works, 4.6 s per run including container start |
| The brr image exists on a Host only if built or loaded there | measured: images are per machine (uncloud builds on the machine) |
| `docker save \| docker -H ssh://… load` moves an image | assumed (Docker docs) |
| A dropped SSH connection closes the container's stdin, so the runner stops the browser | assumed; the runner already treats stdin EOF as stop |

## 4. Current state

Docker-sandboxed browsers run only on brr's own Docker. The runner reaches brr's forwarder through
a Unix socket in the bind-mounted run dir, and Profile/download/extension paths are mounts of brr's
own files. This plan adds Hosts, a framed stdio protocol so one connection carries the browser pipe,
proxy tunnels and file transfers (used for local containers too, replacing the `proxy.sock` mount),
profile/download/build transfer, image transfer, and placement.

## 5. Architecture

```mermaid
flowchart LR
  subgraph brr[brr daemon]
    A[admission: pick Host] --> E[engine: docker -H <host> run -i]
    T[stdio framing: demux] --> P[CDP/Juggler mux]
    T --> F[forwarder.serve(stream)]
    T --> X[files: profile in/out, downloads]
  end
  subgraph H[Host: container brr-<id>, --network none]
    R[brr runner] --> B[browser]
    R --> L[relay 127.0.0.1:3128]
  end
  E <-- ssh: stdin/stdout frames --> R
```

- **Frames.** Container stdio carries frames `[stream u32 BE][kind u8][len u32 BE][payload]`, kinds
  `Open`, `Data`, `Close`. Stream 0 is the browser pipe (bytes as today). Each TCP connection to the
  runner's relay opens a new odd stream; the daemon hands it to the run's forwarder
  (`Forwarder::serve`). Even streams > 0 are daemon-opened file transfers. Payload ≤ 64 KiB; a named
  per-stream window caps buffering. Local docker runs use the same framing (one code path; the
  `proxy.sock` mount goes away).
- **Paths.** On a remote Host nothing of brr's filesystem exists. Each run gets a named volume
  `brr-run-<id>` (labelled with the owner) mounted (by `volume-subpath`) at brr's `user_data_dir`,
  `download_dir` and `run_dir` paths, so argv and CDP paths stay unchanged. Docker mounts only an
  existing subpath and a new volume belongs to root, so a helper container (`install -d -o uid -g gid`)
  makes the three folders first. Before the browser starts, the daemon streams a tar of the Profile
  folder and of the run dir (extension copies) on file streams; the runner unpacks them, then starts
  the browser.
- **Stop.** After the browser exits, the runner streams a tar of the Profile folder back before it
  exits; the daemon unpacks it into `<profile>.incoming` and swaps it in only when the tar
  completed (a dropped connection leaves the Profile as it was before the run); `Exited` is emitted
  after the swap. Ephemeral Profiles skip the copy-back. The volume is removed with the container.
- **File streams.** The daemon opens even streams (2, 4, …); the runner opens odd streams (1, 3, …).
  `Open` payloads are JSON (`sandbox::files::Request`): the daemon sends `"spec"` first on every run
  (data: the `RunnerSpec`), then on a Host `{"put": "<dir>"}` per folder (data: a tar the runner
  unpacks into `<dir>`; it answers `done` or an error text) and `"start"`; `{"get": "<path>"}` is
  answered with the file's size (u64 BE) and its bytes. The runner opens `"profile"` for the
  copy-back; a proxy tunnel's payload is empty. Every tar holds only folders and regular files and
  ends in a `.brr-complete` entry, so a tar cut between entries is told from a whole one; transfers
  are capped at 4 GiB. The runner only accepts paths under the run's mounted dirs.
- **Downloads.** When a download completes, the daemon `get`s the file into the local download dir;
  the existing Artifact flow continues from there.
- **Builds.** The browser must exist on the Host: builds that live inside the image (its
  `/usr/bin/chromium`) run as-is; the browser folder, Camoufox fonts and add-ons from brr's machine
  are copied once to a Host volume `brr-build-<sha256 of the path>` (tar into a helper container,
  skipped when the volume has `.brr-complete`) and mounted read-only at the same path.
- **Image.** Before the first launch on a Host the daemon checks `docker image inspect <image>`;
  missing → `docker save | docker -H … load` once (≈1.4 GB). Both checks are cached per Host for the
  daemon's lifetime.
- **Placement.** Admission considers `local` (brr's own Docker, when the daemon may run browsers) and
  every enabled Host. Candidates have room (`max_instances`); among them the one with the most free
  memory wins (Host memory from `ssh … cat /proc/meminfo` through the same SSH target, cached 30 s,
  refreshed only when admitting). A Host that fails probing is skipped and reported.
- **Process view.** `instance.host_id` records where a run is. Container removal, orphan sweep and
  RSS (`docker stats`) run against the run's Host.

## 6. Flows

**Add Host:** Settings → Hosts → Add: name, `ssh://user@host[:port]`, max browsers → brr probes
`docker -H … version`, memory, image presence → saved; status shown (ok / unreachable / image missing).
SSH keys come from the daemon user's `~/.ssh` (agent or key files); brr stores no SSH secrets.

**Launch:** admit → pick Host → (image check) → (build sync) → `docker -H <host> run -i … -v brr-run-<id>…`
→ upload Profile tar → runner starts browser → CDP over stream 0.

**Stop:** Browser.close → browser exits → runner sends Profile tar → container exits → daemon swaps
Profile → `docker -H <host> rm -f`, volume removed.

**Host lost mid-run:** stdin EOF on the Host side stops the browser; brr sees the transport close →
run ends `crashed` (restart policy applies, possibly on another Host); Profile keeps its pre-run copy.

## 7. Acceptance

```
AC-1  As an operator, I want to add a machine as a Host, so that brr can run more browsers.
      GIVEN ssh access WHEN I add ssh://alex@ssh.aresa.me THEN it shows reachable, its memory and max browsers
      Verify by: live test from the local daemon
AC-2  As an agent, I want launches spread over Hosts, so that one machine isn't overloaded.
      GIVEN local full (max reached) WHEN I launch THEN the browser runs on the Host and every tool works
      Verify by: live test: navigate, snapshot, screenshot, pdf, download, upload, evaluate
AC-3  As a user, I want my persistent Profile's cookies to survive a remote run.
      GIVEN a cookie set on a Host WHEN the run stops and I relaunch locally THEN the cookie is there
      Verify by: live test
AC-4  As a user with a proxy, I want remote browsers to use it without the Host seeing credentials.
      GIVEN a proxied Profile on a Host THEN the exit IP is the proxy's and the Host's container env/volume has no credential
      Verify by: live test
AC-5  As an operator, I want no leftovers on Hosts.
      GIVEN runs on a Host WHEN they stop (normal, crash, brr killed) THEN no brr- container or brr-run- volume remains after brr restarts
      Verify by: live test
AC-6  GIVEN a Host is unreachable WHEN a launch is admitted THEN another Host or local is used, and the Host shows the error
      Verify by: integration test with a fake docker URL
```

## 8. Types

```rust
// brr-core model.rs
pub struct Host {
    pub id: HostId,
    pub name: String,
    pub docker_url: String,      // ssh://user@host[:port]
    pub max_instances: u32,
    pub enabled: bool,
    pub created_at: i64,
}
pub struct HostStatus {          // host.list / host.get output adds this
    pub reachable: bool,
    pub error: Option<String>,
    pub mem_total_mb: Option<u64>,
    pub mem_available_mb: Option<u64>,
    pub running: u32,
    pub image_present: Option<bool>,
    pub checked_at: Option<i64>,
}
// Instance gains: pub host_id: Option<HostId>   // None = local

// brr-engine sandbox
pub struct DockerTarget { /* + */ pub docker_url: Option<String> }
// Some = a remote Host: run volume + tar transfer instead of mounts of brr's paths

// brr-proto::frames (shared by daemon and runner; brr-engine already depends on brr-proto)
pub enum FrameKind { Open = 1, Data = 2, Close = 3, Window = 4 } // Window: credit grant, u32 BE bytes; 1 MiB window per stream; Close = half-close
pub struct Frame { pub stream: u32, pub kind: FrameKind, pub payload: Bytes }
```

Migration 0009: `host` table; `instance.host_id TEXT REFERENCES host(id) ON DELETE SET NULL`.

Ops: `host.list`, `host.get`, `host.create{name, docker_url, max_instances}`, `host.update`,
`host.delete` (refused while it runs browsers), `host.check` (probe now). CLI `brr host …`.
Settings: none new (Hosts are objects).

## 9. Struck

- ~~Profiles pinned to a Host~~, ~~ephemeral-only remote~~: user chose copy in/out.
- ~~Forwarder on each Host~~: user chose traffic via brr; credentials never leave brr.
- ~~Manual placement~~: user chose automatic by free memory.
- ~~brr agent daemon on Hosts~~: Docker over SSH needs nothing installed but Docker.
- ~~Registry for the image~~: save/load needs no registry; a `sandbox.image` that a Host can pull works too.
- ~~Sync of the whole data dir (NFS/rsync)~~: per-run tar is simpler and bounded.

## 10. Slices

| Bead | Slice |
|---|---|
| hosts.1 | Frame protocol: runner + daemon demux; local docker switches to it (proxy over frames, no proxy.sock) |
| hosts.2 | Remote paths: run volume, Profile tar in/out, download fetch, build/font sync, image transfer |
| hosts.3 | Host model, store, ops, CLI, probe (docker version, meminfo, image), status |
| hosts.4 | Placement in admission; per-run Host for kill/sweep/rss |
| hosts.5 | Web: Settings → Hosts; Host column in Browsers/History |
| hosts.6 | Live test with ssh.aresa.me as a Host of the local daemon (AC-1..AC-5) |
