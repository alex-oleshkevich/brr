#!/usr/bin/env bash
# Release build with the web UI and companion extension embedded, copied to DIR (default ~/.local/bin).
set -euo pipefail

dir="${1:-$HOME/.local/bin}"

bun install --cwd web --frozen-lockfile
bun run --cwd web build
bun run --cwd extension build.ts
cargo build -p brr --release

# install replaces the file instead of writing into it, so a running daemon keeps its old binary.
install -Dm755 target/release/brr "$dir/brr"
echo "installed $dir/brr ($("$dir/brr" version))"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "note: $dir is not on PATH" ;;
esac
if "$dir/brr" daemon status >/dev/null 2>&1; then
  echo "a daemon is running the old binary; restart it: brr daemon stop && brr daemon start"
fi
