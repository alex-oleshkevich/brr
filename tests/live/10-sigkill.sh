#!/usr/bin/env bash
# S1: kill -9 the daemon; the next start reconciles every browser it left running.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
new_profile eph ephemeral

ids=() dirs=()
for _ in 1 2 3 4 5; do
    launch eph
    ids+=("$(jqv '.instance.id')")
    dirs+=("$(jqv '.instance.data_dir')")
done
CHECKS=$((CHECKS + 1))
[[ $(marker_pids "${ids[@]}" | wc -l) -ge 5 ]] || fail "expected browsers carrying the 5 instance ids before the kill"

kill -9 "$(daemon_pid)"
wait_until '! daemon_alive' 5
rm -f "$SOCK"

start_daemon

wait_until 'no_marker_pids "${ids[@]}"' 10
assert_eq "$(marker_pids "${ids[@]}" | wc -l)" 0 "no process carries an instance id"

for i in "${!ids[@]}"; do
    rest GET "/v1/instances/${ids[$i]}"
    assert_json '.state == "stopped" and .end_reason == "daemon_restart"' "instance ${ids[$i]} reconciled"
    CHECKS=$((CHECKS + 1))
    [[ ! -e ${dirs[$i]} ]] || fail "ephemeral dir survives: ${dirs[$i]}"
done

for id in "${ids[@]}"; do
    assert_log "instance_id=$id from=Running to=Stopped trigger=Reconcile"
    assert_log "stale instance reconciled instance_id=$id"
done

finish "5 instances reconciled after kill -9, no browsers or ephemeral dirs left"
