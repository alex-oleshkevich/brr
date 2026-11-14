#!/usr/bin/env bash
# V3: a Live browser attached by CDP endpoint; brr never kills it.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome
start_test_chrome
http_server "$M7_FIXTURES"
base=http://127.0.0.1:$HTTP_PORT

chrome_alive() { kill -0 "$TEST_CHROME_PID" 2>/dev/null; }
cdp="http://127.0.0.1:$TEST_CHROME_PORT"
attach_body() { jq -nc --arg u "$1" '{cdp_url: $u}'; }

# Two fixture pages the user "has open".
for page in one two; do
    curl -fsS --max-time 10 -X PUT "$cdp/json/new?$base/$page.html" >/dev/null
done

# --- errors first
rest POST /v1/instances/attach "$(attach_body http://127.0.0.1:1)"
assert_status 400 "attach to a closed port"
assert_json '.code == "InvalidArgument" and .details.field == "cdp_url"'
rest POST /v1/instances/attach "$(jq -nc --arg u "$cdp" '{cdp_url: $u, install_name: "chromium"}')"
assert_status 400 "both cdp_url and install_name"
assert_json '.code == "InvalidArgument"'

# --- attach
rest POST /v1/instances/attach "$(attach_body "$cdp")"
assert_status 200 "attach by cdp_url"
inst=$(jqv '.instance.id')
assert_json '.instance.kind == "live" and .instance.state == "attached"' "a Live Instance, attached"
assert_json '[.tabs[] | select(.url | endswith("one.html") or endswith("two.html"))] | length == 2' "both fixture pages are Tabs"
assert_json "(.tabs | length) >= 2 and ([.tabs[] | select(.opened_by_brr)] | length) == 0" "no Tab was opened by brr"
tab=$(jq -r '[.tabs[] | select(.url | endswith("one.html"))][0].id' <<<"$BODY")
chrome_alive || fail "the test Chrome died on attach"

# --- leases between two Clients
rest POST /v1/clients '{}'
a=$(jqv '.id')
rest POST /v1/clients '{}'
b=$(jqv '.id')
rest POST "/v1/tabs/$tab/navigate" "$(jq -nc --arg u "$base/two.html" '{url: $u}')" "$a"
assert_status 200 "A navigates a user Tab"
rest POST "/v1/tabs/$tab/navigate" '{"url": "about:blank"}' "$b"
assert_status 423 "B navigates A's Tab"
assert_json '.code == "LeaseHeld"'
rest POST "/v1/tabs/$tab/screenshot" '{}' "$b"
assert_status 200 "B reads A's Tab"

# --- closing: a user Tab is not brr's to close, brr's own Tab is
rest DELETE "/v1/tabs/$tab" "" "$a"
assert_status 400 "closing a user Tab"
assert_json '.code == "InvalidArgument" and .details.field == "tab_id"'
rest POST "/v1/instances/$inst/tabs" "$(jq -nc --arg u "$base/one.html" '{url: $u}')" "$a"
assert_status 200 "tab.open on the Live Instance"
own=$(jqv '.tab.id')
assert_json '.tab.opened_by_brr == true'
rest DELETE "/v1/tabs/$own" "" "$a"
assert_status 200 "closing brr's own Tab"

# --- stop is refused, detach leaves the browser running
rest POST "/v1/instances/$inst/stop" '{}'
assert_status 409 "stop on a Live Instance"
assert_json '.code == "InvalidTransition"'
rest POST "/v1/instances/$inst/detach" '{}'
assert_status 200 "detach"
assert_json '.state == "detached"'
chrome_alive || fail "the test Chrome died on detach"

# --- re-attach gives a new Instance; SIGTERM of the daemon leaves the browser alone
rest POST /v1/instances/attach "$(attach_body "$cdp")"
assert_status 200 "re-attach"
second=$(jqv '.instance.id')
[[ $second != "$inst" ]] || fail "re-attach reused the Instance id"
chrome_alive || fail "the test Chrome died on re-attach"

stop_daemon 30 || fail "the daemon ignored SIGTERM"
chrome_alive || fail "daemon shutdown killed the Live browser"
start_daemon
rest GET "/v1/instances/$second"
assert_status 200 "the Live Instance after a daemon restart"
CHECKS=$((CHECKS + 1))
[[ $(jq -r .state <<<"$BODY") != attached ]] || fail "a Live Instance is attached again after a restart without a connection"
chrome_alive || fail "the test Chrome died after the daemon restarted"

# --- the browser goes away: the Instance becomes lost
rest POST /v1/instances/attach "$(attach_body "$cdp")"
assert_status 200 "attach again"
third=$(jqv '.instance.id')
kill "$TEST_CHROME_PID"
wait_until "[[ \$(instance_state $third) == lost ]]" 10

finish "attach, leases, user Tabs protected, stop 409, detach and daemon SIGTERM leave Chrome alive, lost on exit"
