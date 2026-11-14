#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail

# `system` Builds are registered by detect, never installed.
rest POST /v1/builds '{"source": "system"}'
assert_status 400 "install system"
assert_json '.code == "InvalidArgument" and .details.field == "source"'

rest GET /v1/builds
assert_status 200 build.list
assert_json '.builds == []' "no Builds before detect"

if have_chrome; then
    detect_builds
    assert_json '[.builds[] | select(.source == "system" and .state == "registered")] | length >= 1'
    build_id=$(jqv '.builds[0].id')
    rest GET "/v1/builds/$build_id"
    assert_status 200 build.get
    assert_json '.source == "system"'
    rest GET /v1/builds
    assert_json "[.builds[].id] | index(\"$build_id\")" "detected Build is listed"
    rest DELETE "/v1/builds/$build_id"
    assert_status 400 "system Builds are detected, not removable"
    assert_json '.code == "InvalidArgument" and .details.field == "build_id"'
    rest GET "/v1/builds/$build_id"
    assert_status 200 "the Build is still registered"
    detected="detect ran"
else
    detected="detect skipped (no browser on PATH)"
fi

if [[ ${BRR_LIVE_NETWORK-} == 1 ]]; then
    sse=$SCRATCH/sse.txt
    curl -sN --unix-socket "$SOCK" -H 'Last-Event-ID: 0' http://localhost/v1/events/stream >"$sse" &
    rest POST /v1/builds '{"source": "cft", "channel": "stable"}'
    assert_status 200 "install cft stable"
    build_id=$(jqv '.id')
    wait_until 'rest GET "/v1/builds/$build_id"; [[ $(jq -r .state <<<"$BODY") == installed ]]' 600
    CHECKS=$((CHECKS + 1))
    grep -q '^event: build.install_progress' "$sse" || fail "no build.install_progress on SSE"
    network=installed
else
    network="cft install skipped (BRR_LIVE_NETWORK unset)"
fi

finish "$detected; $network"
