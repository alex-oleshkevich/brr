#!/usr/bin/env bash
# T4: /cdp speaks raw CDP, one exclusive Lease per connection, one Instance's targets per client.
# env: BRR_LEASE__EXCLUSIVE_WAIT_S=1
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
new_profile eph ephemeral
ws=ws://$BIND/cdp?profile=eph

frames_with() { # FILE JQ: true when some frame of FILE satisfies JQ
    jq -e "select($2)" "$1" >/dev/null 2>&1
}
connect_lease() {
    rest GET /v1/leases
    jq -e '[.leases[] | select(.client_kind == "connect" and .exclusive == true)] | length >= 1' >/dev/null <<<"$BODY"
}
connect_lease_gone() {
    rest GET /v1/leases
    jq -e '[.leases[] | select(.client_kind == "connect")] | length == 0' >/dev/null <<<"$BODY"
}

# One connection: getTargets answers with a page, and the connection holds an exclusive Lease.
cdp_ws "$ws" '[{"method": "Target.getTargets"}, {"hold_ms": 6000}]' >"$SCRATCH/one.out" &
one=$!
wait_until connect_lease 30
rest GET /v1/leases
first_instance=$(jqv '[.leases[] | select(.client_kind == "connect")][0].instance_id')
wait_until "frames_with $SCRATCH/one.out '.id == 1'" 15
CHECKS=$((CHECKS + 1))
jq -e 'select(.id == 1) | .result.targetInfos | map(select(.type == "page")) | length >= 1' "$SCRATCH/one.out" >/dev/null ||
    fail "Target.getTargets over /cdp shows no page: $(head -c 400 "$SCRATCH/one.out")"

# A second connection sees its own targets only: a target made on the first is not listed.
marker="data:text/html,<title>cdp-$(ulid)</title>"
cdp_ws "$ws" "$(jq -nc --arg u "$marker" '[{"method": "Target.createTarget", "params": {"url": $u}}, {"hold_ms": 4000}]')" >"$SCRATCH/created.out" &
created=$!
wait_until "frames_with $SCRATCH/created.out '.id == 1 and .result.targetId'" 30
cdp_ws "$ws" '[{"method": "Target.getTargets"}]' >"$SCRATCH/two.out"
CHECKS=$((CHECKS + 1))
jq -e 'select(.id == 1) | .result.targetInfos | length >= 1' "$SCRATCH/two.out" >/dev/null || fail "second connection saw no targets"
CHECKS=$((CHECKS + 1))
! grep -qF "$marker" "$SCRATCH/two.out" || fail "the second connection sees a target of another connection"
wait "$created" || true
wait "$one" || true
wait_until connect_lease_gone 15

# Browser.close from a /cdp Client is its stop request: stop, never crashed.
cdp_ws "$ws" '[{"method": "Target.getTargets"}, {"hold_ms": 3000}, {"method": "Browser.close"}]' >"$SCRATCH/close.out" &
closer=$!
wait_until connect_lease 30
rest GET /v1/leases
closing_instance=$(jqv '[.leases[] | select(.client_kind == "connect")][0].instance_id')
wait "$closer" || true
wait_until '[[ $(instance_state "$closing_instance") == stopped ]]' 30
trail=$(state_trail "$closing_instance" | tr '\n' ' ')
CHECKS=$((CHECKS + 1))
[[ $trail == *"stopping:stop"* && $trail != *crashed* ]] || fail "Browser.close did not stop through the stop trigger: $trail"

# An exclusive Lease held by someone else: the upgrade is answered 423 after the exclusive wait.
new_profile pers persistent
rest POST /v1/clients '{}'
holder=$(jqv '.id')
rest POST /v1/profiles/pers/launch '{"exclusive": true}' "$holder"
assert_status 200 "launch with an exclusive Lease"
held=$(jqv '.instance.id')
status=$(curl -s -o "$SCRATCH/upgrade.body" -w '%{http_code}' --max-time 20 --http1.1 \
    -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' \
    -H "Sec-WebSocket-Key: $(head -c 16 /dev/urandom | base64)" "http://$BIND/cdp?profile=pers" || true)
assert_eq "$status" 423 "/cdp upgrade against an exclusive Lease"
CHECKS=$((CHECKS + 1))
jq -e '.code == "LeaseHeld"' "$SCRATCH/upgrade.body" >/dev/null || fail "423 body is not LeaseHeld: $(<"$SCRATCH/upgrade.body")"
rest POST "/v1/instances/$held/stop" '{}' "$holder"
assert_status 200 "the holder stops its Instance"

finish "/cdp targets, exclusive connect Lease, isolation, Browser.close stop, 423 on a held Lease"
