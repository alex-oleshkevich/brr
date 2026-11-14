#!/usr/bin/env bash
# Docker's default seccomp profile plus user-namespace syscalls, so Chrome's sandbox works unprivileged.
set -euo pipefail
cd "$(dirname "$0")/.."
MOBY_TAG=docker-v29.9.2
curl -fsSL "https://raw.githubusercontent.com/moby/moby/$MOBY_TAG/vendor/github.com/moby/profiles/seccomp/default.json" \
    | jq '.syscalls += [{"names": ["clone", "clone3", "unshare", "setns"], "action": "SCMP_ACT_ALLOW"}]' \
    > deploy/seccomp-chrome.json
