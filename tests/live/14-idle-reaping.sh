#!/usr/bin/env bash
# §7.3: Tab reaping, the last Tab blanked (I2), Instance idle -> reaping -> stopped, activity in grace.
# Idle timeouts count whole minutes, so this script runs a few minutes.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
set_settings '{"idle.tab_min": 1, "idle.instance_min": 2, "idle.grace_s": 5}'
new_profile eph ephemeral

launch eph
one=$(jqv '.instance.id')
tab1=$(jqv '.tab.id')
open_tab "$one"
tab2=$TAB_ID

# A second Instance for the activity-during-grace case, idle on the same clock.
launch eph
two=$(jqv '.instance.id')
two_tab=$(jqv '.tab.id')

# 2./3. (checked in the window before the Instance idles) Exactly one of the two Tabs is reaped after its warning; the other is the last Tab, so it is
# blanked and left idle, never closed (I2).
reaped_tab() {
    events_for "$one"
    jq -e '[.events[] | select(.kind == "tab.closed" and .data.reason == "reaped")] | length >= 1' >/dev/null <<<"$BODY"
}
wait_until reaped_tab 200
assert_json '[.events[] | select(.kind == "tab.closed" and .data.reason == "reaped")] | length == 1' "one Tab reaped"
reaped=$(jqv '[.events[] | select(.kind == "tab.closed" and .data.reason == "reaped")][0].data.tab_id')
assert_json "[.events[] | select(.kind == \"tab.reaping_warning\" and .data.tab_id == \"$reaped\")] | length == 1" "tab warning before the close"
if [[ $reaped == "$tab1" ]]; then kept=$tab2; else kept=$tab1; fi
assert_json "[.events[] | select(.kind == \"tab.closed\" and .data.tab_id == \"$kept\" and .data.reason == \"reaped\")] | length == 0" "the last Tab is never reaped"
kept_blank() {
    rest GET "/v1/tabs/$kept"
    jq -e '.url == "about:blank" and .state == "idle" and .closed_at == null' >/dev/null <<<"$BODY"
}
wait_until kept_blank 30

# 5. A call during the reaping grace brings the Instance back.
has_warning() {
    events_for "$two"
    jq -e '[.events[] | select(.kind == "instance.reaping_warning")] | length >= 1' >/dev/null <<<"$BODY"
}
wait_until has_warning 300
rest POST "/v1/tabs/$two_tab/navigate" '{"url": "data:text/html,<title>busy</title>"}'
assert_status 200 "navigate during the grace period"
trail=$(state_trail "$two" | tr '\n' ' ')
CHECKS=$((CHECKS + 1))
[[ $trail == *"reaping:reap"*"running:activity"* ]] || fail "expected reaping then running:activity in: $trail"
assert_eq "$(instance_state "$two")" running "Instance two is running again"
stop_instance "$two"

# 4. The Instance itself: idle, reaping with a warning, stopping, stopped.
wait_until '[[ $(instance_state "$one") == stopped ]]' 300
rest GET "/v1/instances/$one"
assert_json '.end_reason == "reaped"'
trail=$(state_trail "$one" | tr '\n' ' ')
CHECKS=$((CHECKS + 1))
[[ $trail == *"idle:idle_timeout reaping:reap stopping:grace_over stopped:exited"* ]] || fail "unexpected trail: $trail"
events_for "$one"
assert_json '[.events[] | select(.kind == "instance.reaping_warning")] | length == 1' "instance.reaping_warning"

finish "tab reaped, last tab blanked, instance reaped, activity cancels the grace"
