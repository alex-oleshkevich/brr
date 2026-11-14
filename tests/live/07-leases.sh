#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
new_profile eph ephemeral

rest POST /v1/clients '{}'
a=$(jqv '.id')
rest POST /v1/clients '{}'
b=$(jqv '.id')

# 1. A holds the launch Lease: B is blocked from writing, not from reading.
launch eph "$a"
instance=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
assert_json ".lease.client_id == \"$a\""

rest POST "/v1/tabs/$tab/navigate" '{"url": "about:blank"}' "$b"
assert_status 423 "B navigates A's tab"
assert_json ".code == \"LeaseHeld\" and .details.client_id == \"$a\""
rest GET "/v1/tabs/$tab" "" "$b"
assert_status 200 "B reads A's tab"

# 2. An exclusive Lease locks the whole Instance.
rest POST /v1/leases "{\"instance_id\": \"$instance\", \"exclusive\": true}" "$a"
assert_status 200 "A takes an exclusive lease"
assert_json '.exclusive == true'
rest POST "/v1/instances/$instance/tabs" '{}' "$b"
assert_status 423 "B opens a tab on A's exclusive instance"
assert_json '.code == "LeaseHeld"'

# 3. Ending A releases everything it held.
rest GET "/v1/leases?client_id=$a"
assert_json '.leases | length >= 1' "A holds leases"
rest DELETE "/v1/clients/$a"
assert_status 200 client.end
rest GET /v1/leases
assert_json "[.leases[] | select(.client_id == \"$a\")] | length == 0" "A's leases are gone"
rest POST "/v1/instances/$instance/tabs" '{}' "$b"
assert_status 200 "B opens a tab once A is gone"

# 4. A CLI process killed with SIGKILL loses its Lease.
port=$(free_port)
slow_server "$port"
"$BRR_BIN" tab navigate --tab-id "$tab" "http://127.0.0.1:$port/" >/dev/null 2>&1 &
cli=$!
cli_lease() {
    rest GET /v1/leases
    jq -e '[.leases[] | select(.client_kind == "cli")] | length >= 1' >/dev/null <<<"$BODY"
}
wait_until cli_lease 10

rc=0
brr tab navigate --tab-id "$tab" about:blank >/dev/null 2>&1 || rc=$?
assert_eq "$rc" 6 "CLI LeaseHeld exit code"

kill -9 "$cli"
wait "$cli" 2>/dev/null || true
no_cli_lease() {
    rest GET /v1/leases
    jq -e '[.leases[] | select(.client_kind == "cli")] | length == 0' >/dev/null <<<"$BODY"
}
wait_until no_cli_lease 5

# B holds the Tab it opened: a plain stop by another Client is refused; stop_instance forces it (LC9).
rest POST /v1/clients '{}'
c=$(jqv '.id')
rest POST "/v1/instances/$instance/stop" '{}' "$c"
assert_status 423 "plain stop while another Client holds a Lease"
assert_json ".code == \"LeaseHeld\" and .details.client_id == \"$b\""

rest DELETE "/v1/clients/$b"
stop_instance "$instance"

finish "LeaseHeld 423, read-only access, exclusive, DELETE-client and CLI-SIGKILL release"
