#!/usr/bin/env bash
# I4/§18: pinned Tabs and pinned Profiles are never reaped; the last Tab of a visible browser is blanked, not closed.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
set_settings '{"idle.tab_min": 1, "idle.instance_min": 1, "idle.grace_s": 5}'

# Headless: three Tabs, one pinned. Its Instance gets a long idle timeout so only Tabs are reaped.
new_profile hl ephemeral
rest GET /v1/profiles/hl
rest PATCH /v1/profiles/hl "$(jq -c '{settings: (.settings | .idle.instance_min = 30)}' <<<"$BODY")"
assert_status 200 "per-profile instance_min"
launch hl
headless=$(jqv '.instance.id')
first=$(jqv '.tab.id')
open_tab "$headless"
pinned=$TAB_ID
open_tab "$headless"
loose=$TAB_ID
rest POST "/v1/tabs/$pinned/pin" '{}'
assert_status 200 tab.pin
assert_json '.pinned == true'

# Visible: a pinned Profile whose only Tab goes idle.
if has_display; then
    new_profile vis ephemeral chromium-visible
    rest PATCH /v1/profiles/vis '{"pinned": true}'
    assert_status 200 "pin profile"
    rest POST /v1/profiles/vis/launch '{"display": "visible", "url": "about:blank"}'
    assert_status 200 "visible launch"
    visible=$(jqv '.instance.id')
    visible_tab=$(jqv '.tab.id')
fi

both_closed() {
    rest GET "/v1/instances/$headless/tabs"
    jq -e "[.tabs[] | select(.id == \"$first\" or .id == \"$loose\") | select(.closed_at == null)] | length == 0" >/dev/null <<<"$BODY"
}
wait_until both_closed 240
rest GET "/v1/tabs/$pinned"
assert_json '.closed_at == null and .pinned == true' "the pinned Tab survives"
events_for "$headless"
assert_json "[.events[] | select(.kind == \"tab.closed\" and .data.tab_id == \"$pinned\")] | length == 0" "no close event for the pinned Tab"
assert_json '[.events[] | select(.kind == "tab.closed" and .data.reason == "reaped")] | length == 2' "both unpinned Tabs reaped"
stop_instance "$headless"

if has_display; then
    visible_idle() {
        rest GET "/v1/tabs/$visible_tab"
        jq -e '.state == "idle" and .url == "about:blank"' >/dev/null <<<"$BODY" || return 1
        [[ $(instance_state "$visible") == idle ]]
    }
    wait_until visible_idle 240
    trail=$(state_trail "$visible" | tr '\n' ' ')
    CHECKS=$((CHECKS + 1))
    [[ $trail != *reaping* && $trail != *crashed* && $trail != *stopping* ]] || fail "pinned visible Instance left idle: $trail"
    stop_instance "$visible"
    visible_part="visible part ran"
else
    visible_part="visible part skipped (no DISPLAY or WAYLAND_DISPLAY)"
fi

finish "pinned Tab kept, unpinned reaped; $visible_part"
