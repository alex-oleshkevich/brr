#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
new_profile eph ephemeral

launch eph "" 'data:text/html,<title>a</title>'
assert_json '.instance.state == "running"' "instance running"
assert_json '.tab.state == "ready"' "first tab ready"
assert_json '.lease.id | length > 0' "launch returns a Lease"
first=$(jqv '.instance.id')
first_dir=$(jqv '.instance.data_dir')
tab1=$(jqv '.tab.id')
CHECKS=$((CHECKS + 1))
[[ -d $first_dir ]] || fail "ephemeral data dir missing while running: $first_dir"

rest GET "/v1/tabs/$tab1"
assert_json '.title == "a"' "first tab title"

rest POST "/v1/instances/$first/tabs" '{"url": "about:blank"}'
assert_status 200 tab.open
tab2=$(jqv '.tab.id')
rest POST "/v1/tabs/$tab2/navigate" '{"url": "data:text/html,<title>b</title>"}'
assert_status 200 tab.navigate
rest GET "/v1/tabs/$tab2"
assert_json '.title == "b"' "navigate took effect"
rest POST "/v1/tabs/$tab1/switch" '{}'
assert_status 200 tab.switch
rest GET "/v1/instances/$first/tabs"
assert_json '[.tabs[] | select(.state != "closed")] | length == 2' "two tabs"
rest DELETE "/v1/tabs/$tab2"
assert_status 200 tab.close
rest GET "/v1/instances/$first/tabs"
assert_json '[.tabs[] | select(.closed_at == null)] | length == 1' "one tab left"

rest POST "/v1/tabs/$tab1/navigate" '{"url": "http://nonexistent.invalid"}'
assert_status 502 "navigation to a dead host"
assert_json '.code == "NavigationFailed" and (.details.net_error | length > 0)'

stop_instance "$first"
wait_until '[[ $(instance_state "$first") == stopped ]]' 10
rest GET "/v1/instances/$first"
before=$(jq -S . <<<"$BODY")
assert_json '.end_reason == "stop"'
rest POST "/v1/instances/$first/stop" '{}'
assert_status 409 "stop a stopped instance"
assert_json '.code == "InvalidTransition" and .details.from == "stopped" and .details.to == "stopping"'
rest GET "/v1/instances/$first"
assert_eq "$(jq -S . <<<"$BODY")" "$before" "failed transition leaves the row unchanged"
assert_log "instance_id=$first from=Running to=Stopping trigger=Stop"

wait_until '[[ ! -e $first_dir ]]' 10
rc=0
brr instance stop "$first" >/dev/null 2>&1 || rc=$?
assert_eq "$rc" 5 "CLI InvalidTransition exit code"

launch eph
second=$(jqv '.instance.id')
CHECKS=$((CHECKS + 1))
[[ $second != "$first" ]] || fail "relaunch reused instance id"
stop_instance "$second"

finish "launch, tabs, navigation error, stop and InvalidTransition, relaunch"
