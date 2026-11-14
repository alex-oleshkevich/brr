#!/usr/bin/env bash
# M5: await_human, input from the UI without a take over (LC6), human-done, its timeout and error paths.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
http_server "$(dirname "$0")/fixtures/m5"
base=http://127.0.0.1:$HTTP_PORT/live.html
new_profile eph ephemeral

rest POST /v1/clients '{"label": "agent-1"}'
agent=$(jqv '.id')
rest POST /v1/clients '{"kind": "ui", "label": "ui"}'
ui=$(jqv '.id')

launch eph "$agent" "$base"
instance=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
holder=$agent
out=$SCRATCH/ws.log

# The agent asks for a person; the call stays open until the person is done.
(curl -sS --max-time 120 --unix-socket "$SOCK" -X POST -H "Brr-Client: $agent" -H 'content-type: application/json' \
    --data '{"reason": "login", "timeout_ms": 60000}' -o "$SCRATCH/wait.body" -w '%{http_code}' \
    "http://localhost/v1/tabs/$tab/await_human" >"$SCRATCH/wait.status") &
SLOW_PIDS+=($!)

awaiting() {
    rest GET "/v1/tabs/$tab"
    [[ $(jq -r .state <<<"$BODY") == awaiting_human ]]
}
wait_until awaiting 10
rest GET "/v1/events?kinds=tab.awaiting_human&order=asc"
assert_json "[.events[] | select(.data.tab_id == \"$tab\")] | length == 1" "tab.awaiting_human was published"

# The UI Client types without taking the Tab over: the Tab awaits a person, so input is accepted.
ws_client "ws://$BIND/v1/tabs/$tab/screencast?client_id=$ui" "$out" 120
wait_until '[[ $(frames_in "$out") -ge 1 ]]' 5
click_at '#name'
ws_send '{"type":"text","text":"hi"}'
typed() {
    tab_call "$tab" evaluate '{"expression": "document.getElementById(\"name\").value"}' "$agent"
    [[ $(jq -r .result <<<"$BODY") == hi ]]
}
wait_until typed 10
CHECKS=$((CHECKS + 1))
! grep -q '"type":"error"' "$out" || fail "the socket reported an error for input to an awaiting Tab: $(grep '"type":"error"' "$out")"

# human-done releases the waiting call.
rest POST "/v1/tabs/$tab/human-done" '{}' "$ui"
assert_status 200 "human-done"
assert_json '.state == "ready"' "the Tab is ready after human-done"
wait_until '[[ -s "$SCRATCH/wait.status" ]]' 10
assert_eq "$(<"$SCRATCH/wait.status")" 200 "the waiting call returned"
assert_eq "$(jq -r .state "$SCRATCH/wait.body")" ready "the waiting call returned a ready Tab"
ws_close

# human-done on a Tab nobody waits for.
rest POST "/v1/tabs/$tab/human-done" '{}' "$ui"
assert_status 409 "human-done on a ready Tab"
assert_json '.code == "InvalidTransition"'

# Nobody comes: the wait times out and the Tab goes back to ready.
tab_call "$tab" await_human '{"reason": "login", "timeout_ms": 1000}' "$agent"
assert_status 504 "await_human with nobody there"
assert_json '.code == "Timeout"'
rest GET "/v1/tabs/$tab"
assert_json '.state == "ready"' "the Tab is ready again after the timeout"
stop_instance "$instance"
wait_until 'no_marker_pids "$instance"' 15

# The Client the Tab came from has ended: hand-back says so and the Tab stays with the person.
rest POST /v1/clients '{"label": "agent-2"}'
leaver=$(jqv '.id')
launch eph "$leaver" "$base"
instance=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
rest POST "/v1/tabs/$tab/take-over" '{}' "$ui"
assert_status 200 "take over from agent-2"
rest DELETE "/v1/clients/$leaver"
assert_status 200 "agent-2 ends"
rest POST "/v1/tabs/$tab/hand-back" '{}' "$ui"
assert_status 409 "hand-back after the Client ended"
assert_json '.code == "Conflict" and .details.reason == "client_gone"'
rest GET "/v1/tabs/$tab"
assert_json ".lease.client_id == \"$ui\"" "the Tab stays with the person"
stop_instance "$instance"
wait_until 'no_marker_pids "$instance"' 15

finish "await_human, input without take over, human-done, timeout, client_gone, InvalidTransition"
