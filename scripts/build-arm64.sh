#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if ! command -v aarch64-linux-gnu-gcc >/dev/null || ! rustup target list --installed 2>/dev/null | grep -qx aarch64-unknown-linux-gnu; then
    echo "install gcc-aarch64-linux-gnu and rustup target add aarch64-unknown-linux-gnu" >&2
    exit 2
fi
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
cargo build --release -p brr --target aarch64-unknown-linux-gnu "$@"
