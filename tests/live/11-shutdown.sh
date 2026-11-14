#!/usr/bin/env bash
# S2: SIGTERM stops every browser within stop_timeout_s + 10.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
rest GET /v1/settings
stop_timeout=$(jqv '.settings.instance.stop_timeout_s')
new_profile eph ephemeral

ids=()
for _ in 1 2; do
    launch eph
    ids+=("$(jqv '.instance.id')")
done
CHECKS=$((CHECKS + 1))
[[ $(marker_pids "${ids[@]}" | wc -l) -ge 2 ]] || fail "expected browsers carrying the 2 instance ids"

pid=$(daemon_pid)
start=$SECONDS
kill -TERM "$pid"
wait_until '! daemon_alive' "$((stop_timeout + 10))"
echo "daemon exited after $((SECONDS - start))s"
assert_eq "$(marker_pids "${ids[@]}" | wc -l)" 0 "no browser outlives the daemon"
assert_log "shutting down"

start_daemon
for id in "${ids[@]}"; do
    rest GET "/v1/instances/$id"
    assert_json '.state == "stopped" and .end_reason == "shutdown"' "instance $id ended by shutdown"
done

finish "SIGTERM stops 2 browsers within ${stop_timeout}s+10"
