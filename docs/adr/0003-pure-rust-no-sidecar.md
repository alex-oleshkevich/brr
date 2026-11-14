# Pure Rust, no Node/Playwright sidecar

The daemon is a single Rust binary. It speaks CDP to the Chromium family and Juggler to Camoufox itself, both over the same fd 3/4 pipe transport. It ports the Camoufox TypeScript launcher to Rust (fpgen fingerprints, chunked `CAMOU_CONFIG_1..N` and `CAMOU_PREFS_1..N`, `FONTCONFIG_FILE` plus the fonts bundle). It builds Playwright's in-page injected script (Apache-2.0) into the binary for snapshots, element references and selectors on both engines, and ports Playwright's actionability and click-point logic to Rust. We rejected a Node/Playwright sidecar, even though it would have given the official Camoufox launcher and Playwright's automation for free, so that there is one deliverable, one process model and no runtime dependency on Node.

## Consequences
- Juggler is an internal, unversioned protocol tied to Playwright minor versions (Camoufox pins `playwright<1.63`), and Camoufox releases every 1–2 weeks. Supported Camoufox versions are pinned to a range. A new Camoufox release is only allowed after a compatibility test suite passes against it.
- The launcher port is about 6 000 lines of TypeScript (utils, fingerprints, coherence, webgl), checked against Camoufox's golden outputs.
- Engines sit behind a driver interface, so a sidecar-backed driver for one engine stays possible if the cost of keeping up with Juggler becomes too high.
- Playwright's `connect()` speaks Playwright's server protocol, not Juggler. A Playwright connect endpoint for Camoufox would need a Playwright server, so it is out of scope and Camoufox is driven only through brr's own operations.
