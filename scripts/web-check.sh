#!/usr/bin/env bash
set -euo pipefail
bun run --cwd web check "$@"
