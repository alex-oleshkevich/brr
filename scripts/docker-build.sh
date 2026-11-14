#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
docker buildx build --load -f deploy/Dockerfile -t brr:dev "$@" .
