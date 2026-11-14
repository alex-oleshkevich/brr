# brr task runner. This is the entry point for every workflow —
# do not invoke cargo directly, add or fix a recipe here instead.

set shell := ["bash", "-uc"]
set positional-arguments

[doc("list recipes")]
default:
    @just --list --unsorted

# ---------------------------------------------------------------- verify

[doc("type-check")]
check:
    cargo check --workspace --all-targets

[doc("type-check one crate, e.g. just check-crate brr-cdp")]
check-crate CRATE:
    cargo check -p {{ CRATE }} --all-targets

[doc("clippy, warnings are errors")]
lint:
    cargo clippy --workspace --all-targets -- -D warnings

[doc("clippy for one crate")]
lint-crate CRATE:
    cargo clippy -p {{ CRATE }} --all-targets -- -D warnings

[doc("format in place")]
fmt:
    cargo fmt --all

[doc("format one crate in place")]
fmt-crate CRATE:
    cargo fmt -p {{ CRATE }}

[doc("fail if anything is unformatted")]
fmt-check:
    cargo fmt --all --check

[doc("headless tests")]
test:
    cargo test --workspace

[doc("tests of one crate; extra args go to cargo test")]
test-crate CRATE *ARGS:
    cargo test -p "$@"

[doc("everything CI runs")]
verify: fmt-check check lint test web-check ext-test

# ---------------------------------------------------------------- web

[doc("install web deps")]
web-install:
    bun install --cwd web

[doc("build the web UI into web/build (embedded by brr-daemon)")]
web-build:
    bun run --cwd web build

[doc("regenerate web/src/lib/api/schema.d.ts from a running daemon's /openapi.json")]
web-gen-api:
    bun run --cwd web gen:api:file http://${BRR_SERVER__BIND:-127.0.0.1:9898}/openapi.json -o src/lib/api/schema.d.ts

[doc("svelte-check the web UI")]
web-check:
    scripts/web-check.sh

[doc("web UI acceptance: Playwright + axe on a scratch daemon (needs just build, just web-build, BRR_TEST_CHROME; BRR_E2E_URL reuses a running daemon)")]
web-e2e *ARGS:
    bun run --cwd web e2e "$@"

[doc("web UI dev server against a running daemon")]
web-dev:
    bun run --cwd web dev

# ---------------------------------------------------------------- extension

[doc("build the companion extension into extension/dist")]
ext-build:
    scripts/ext-build.sh

[doc("test the companion extension; BRR_TEST_CHROME=<chrome binary> adds the browser load test")]
ext-test:
    bun test --cwd extension

# ---------------------------------------------------------------- build and run

[doc("build, debug")]
build:
    cargo build -p brr

[doc("build, release")]
build-release:
    cargo build -p brr --release

[doc("release build with the web UI embedded, installed to ~/.local/bin (or the given folder)")]
install *ARGS:
    scripts/install.sh "$@"

[doc("build, release, aarch64 (needs gcc-aarch64-linux-gnu and the rustup target)")]
build-release-arm64:
    scripts/build-arm64.sh

[doc("build the Docker image brr:dev; extra args go to buildx, e.g. --platform linux/arm64")]
docker-build *ARGS:
    scripts/docker-build.sh "$@"

[doc("regenerate deploy/seccomp-chrome.json from moby's default profile (needs network and jq)")]
docker-seccomp:
    scripts/docker-seccomp.sh

[doc("run brr; arguments go to brr, e.g. just run profile list")]
run *ARGS:
    @cargo run -q -p brr -- "$@"

[doc("rebuild Playwright's injected script and vendored data into crates/brr-engine/assets")]
injected-script:
    scripts/injected-script.sh "$@"

[doc("vendor the pinned Camoufox launcher data into crates/brr-camoufox/data (needs network and bun)")]
camoufox-data *ARGS:
    scripts/camoufox-data.sh "$@"

[doc("vendor the pinned DuckDuckGo autoconsent standalone bundle into crates/brr-content/vendor (needs network)")]
vendor-autoconsent *ARGS:
    scripts/vendor-autoconsent.sh "$@"

[doc("record the Camoufox launcher goldens into crates/brr-camoufox/tests/fixtures (needs network and python3)")]
camoufox-golden *ARGS:
    scripts/camoufox-golden.sh "$@"

[doc("live tests against a scratch daemon, e.g. just live 01-instances")]
live *ARGS:
    scripts/live.sh "$@"
