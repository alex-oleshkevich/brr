#!/usr/bin/env bash
# §17.2: adaptive admission under contention; every admitted decision satisfies the formula.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
new_profile eph ephemeral

rest GET /v1/system/info
free=$(jqv '.mem_available_mb')
load=$(jqv '.load_per_core')
min_free=$((free > 750 ? free - 750 : 0))
# The load ceiling is derived from the host so the test measures memory, not the machine's busyness.
max_load=$(jq -n "($load * 2 + 2)")

rest PUT /v1/limits "$(jq -nc --argjson f "$min_free" --argjson l "$max_load" '{
    mode: "adaptive", max_instances: 100, max_instances_per_engine: {chromium: 100, camoufox: 4},
    adaptive: {min_free_ram_mb: $f, max_load_per_core: $l, reserve_mb: {chromium_headless: 300}},
    admission: {queue_timeout_s: 20}}')"
assert_status 200 "adaptive limits"
assert_json '.mode == "adaptive"'

mkdir -p "$SCRATCH/l"
launch_and_stop() { # N: one caller; stops its Instance as soon as the launch returns 200
    local n=$1 out=$SCRATCH/l/$n.json status
    status=$(curl -s -o "$out" -w '%{http_code}' --max-time 90 --unix-socket "$SOCK" -X POST \
        -H 'content-type: application/json' -d '{"url": "about:blank"}' http://localhost/v1/profiles/eph/launch) || status=000
    echo "$status" >"$SCRATCH/l/$n.status"
    if [[ $status == 200 ]]; then
        curl -s -o /dev/null --max-time 60 --unix-socket "$SOCK" -X POST -H 'content-type: application/json' \
            -d '{}' "http://localhost/v1/instances/$(jq -r .instance.id "$out")/stop"
    fi
}
for n in $(seq 1 20); do
    launch_and_stop "$n" &
done
wait

ok=0 refused=0
for n in $(seq 1 20); do
    status=$(<"$SCRATCH/l/$n.status")
    case $status in
    200) ok=$((ok + 1)) ;;
    429)
        jq -e '.code == "CapExceeded"' "$SCRATCH/l/$n.json" >/dev/null || fail "429 without CapExceeded: $(<"$SCRATCH/l/$n.json")"
        refused=$((refused + 1))
        ;;
    *) fail "launch $n answered $status: $(<"$SCRATCH/l/$n.json")" ;;
    esac
done
CHECKS=$((CHECKS + 1))
((ok >= 1)) || fail "no launch was admitted"

rest GET '/v1/events?kinds=admission.decided&order=asc&limit=1000'
decisions=$BODY
admitted=$(jq '[.events[] | select(.data.outcome == "admitted")] | length' <<<"$decisions")
queued=$(jq '[.events[] | select(.data.outcome == "queued")] | length' <<<"$decisions")
rejected=$(jq '[.events[] | select(.data.outcome == "rejected")] | length' <<<"$decisions")
violations=$(jq --argjson f "$min_free" --argjson l "$max_load" '[.events[] | select(.data.outcome == "admitted")
    | select((.data.inputs.free_ram_mb - .data.inputs.reserved_mb < $f) or (.data.inputs.load_per_core > $l))] | length' <<<"$decisions")
assert_eq "$violations" 0 "admitted decisions that break free_ram_mb - reserved_mb >= min_free_ram_mb and load ceiling"
CHECKS=$((CHECKS + 1))
((admitted >= 1)) || fail "no admitted decision recorded"
CHECKS=$((CHECKS + 1))
((queued >= 1)) || fail "no queued decision: the host had room for all 20 (free ${free} MB, min_free ${min_free} MB)"

# The daemon log carries each decision with its inputs.
assert_log "admission decided"
logged=$(grep -c "admission decided" "$XDG_STATE_HOME/brr/daemon.log" || true)
CHECKS=$((CHECKS + 1))
((logged >= admitted)) || fail "daemon.log has $logged admission lines for $admitted admitted decisions"

echo "results: 200=$ok 429=$refused; decisions: admitted=$admitted queued=$queued rejected=$rejected; min_free=${min_free}MB"
finish "20 launches: $ok admitted, $refused refused with 429, $queued queued decisions; formula held on all $admitted admissions"
