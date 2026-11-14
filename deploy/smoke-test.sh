#!/usr/bin/env bash
# Runs brr:dev (build it first: just docker-build) under both sandbox policies; see deploy/README.md.
set -euo pipefail
cd "$(dirname "$0")/.."

containers=()
trap 'for c in "${containers[@]}"; do docker rm -f "$c" >/dev/null 2>&1 || true; done' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# $1 name, remaining args: extra docker run options. Sets BASE to the published API address.
start() {
    local name=$1; shift
    docker run -d --name "$name" --shm-size=2g -p 127.0.0.1::9898 "$@" brr:dev >/dev/null
    containers+=("$name")
    BASE=http://127.0.0.1:$(docker port "$name" 9898/tcp | head -1 | sed 's/.*://')
    for _ in $(seq 20); do
        curl -fsS --max-time 5 -o /dev/null "$BASE/v1/system/info" 2>/dev/null && return
        sleep 1
    done
    fail "$name: no answer from /v1/system/info within 20 s"
}

api() { curl -fsS --max-time 30 -H 'content-type: application/json' "$@"; }

# $1 container name, $2 profile name, $3 JSON array for the Profile's args
launch_and_check() {
    local name=$1 profile=$2 args=$3 settings state pid
    api -X POST "$BASE/v1/profiles" -d "{\"name\":\"$profile\",\"template\":\"chromium-headless\"}" >/dev/null
    settings=$(api "$BASE/v1/profiles/$profile" | jq -c --argjson args "$args" '{settings: (.settings | .args = $args)}')
    api -X PATCH "$BASE/v1/profiles/$profile" -d "$settings" >/dev/null
    pid=$(api -X POST "$BASE/v1/profiles/$profile/launch" -d '{}' | jq -r .instance.pid)
    for _ in $(seq 30); do
        state=$(api "$BASE/v1/instances" | jq -r '.instances[0].state')
        [ "$state" = running ] && break
        [ "$state" = failed ] && break
        sleep 1
    done
    [ "$state" = running ] || fail "$name: instance is $state, log: $(docker logs --tail 20 "$name" 2>&1)"
    docker exec "$name" cat "/proc/$pid/cmdline" | tr '\0' '\n' | grep -qx -- --disable-dev-shm-usage \
        || fail "$name: browser args lack --disable-dev-shm-usage"
    echo "ok: $name launched Chromium (pid $pid) with --disable-dev-shm-usage"
}

echo "== policy A: seccomp profile allowing user namespaces"
start brr-smoke-a --security-opt seccomp=deploy/seccomp-chrome.json
api -X POST "$BASE/v1/builds/detect" | jq -e '.builds | any(.source == "system" and .engine == "chromium")' >/dev/null \
    || fail "detect did not register Debian chromium"
docker exec brr-smoke-a brr doctor --json | jq -e '
    [.checks[] | select(.name == "xvfb" or .name == "dev_shm" or .name == "docker") | .status] == ["ok", "ok", "ok"]' >/dev/null \
    || fail "doctor: xvfb, dev_shm and docker must be ok"
launch_and_check brr-smoke-a smoke '[]'

echo "== policy B: no seccomp option, --no-sandbox in the Profile"
start brr-smoke-b
api -X POST "$BASE/v1/builds/detect" >/dev/null
launch_and_check brr-smoke-b smoke '["--no-sandbox"]'

echo "smoke test passed"
