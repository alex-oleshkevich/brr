#!/usr/bin/env bash
# M5: the screencast WebSocket: frames, LC6 input rules, take over / hand back, dialogs, closing.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
http_server "$(dirname "$0")/fixtures/m5"
base=http://127.0.0.1:$HTTP_PORT/live.html

rest POST /v1/clients '{"label": "agent-1"}'
agent=$(jqv '.id')
rest POST /v1/clients '{"kind": "ui", "label": "ui"}'
ui=$(jqv '.id')

# title_is TITLE: the page's own document.title (tab.get's title does not follow a script's change).
title_is() {
    tab_call "$tab" evaluate '{"expression": "document.title"}' "$holder"
    [[ $(jq -r .result <<<"$BODY") == "$1" ]]
}

# live_view_checks ENGINE_LABEL PROFILE: the whole walk for one Profile.
live_view_checks() {
    local label=$1 profile=$2
    out=$SCRATCH/$label-ws.log
    launch "$profile" "$agent" "$base"
    instance=$(jqv '.instance.id')
    tab=$(jqv '.tab.id')
    holder=$agent

    # A viewer gets frames.
    ws_client "ws://$BIND/v1/tabs/$tab/screencast?client_id=$ui" "$out" 120
    wait_until '[[ $(frames_in "$out") -ge 1 ]]' 5

    # LC6: input without the Lease or a take over is refused.
    ws_send '{"type":"mouse","event":"down","x":5,"y":5,"button":"left","click_count":1}'
    wait_until 'grep -q "\"code\":\"LeaseHeld\"" "$out"' 5

    # Take over.
    rest POST "/v1/tabs/$tab/take-over" '{}' "$ui"
    assert_status 200 "take over as the UI client"
    assert_json ".lease.client_id == \"$ui\"" "the UI client holds the Lease"
    rest GET "/v1/tabs/$tab"
    assert_json ".lease.client_id == \"$ui\" and .lease.transferred_from_client_id == \"$agent\"" "tab.get names where the Lease came from"
    rest GET "/v1/events?kinds=lease.transferred&order=asc"
    assert_json "[.events[] | select(.data.from_client_id == \"$agent\" and .data.to_client_id == \"$ui\")] | length == 1" "lease.transferred"
    holder=$ui

    # A click at the button's frame position reaches the page.
    click_at '#go'
    wait_until "title_is clicked" 10

    # A key reaches the focused input.
    click_at '#name'
    ws_send '{"type":"key","event":"down","key":"a","code":"KeyA","key_code":65,"text":"a"}'
    ws_send '{"type":"key","event":"up","key":"a","code":"KeyA","key_code":65}'
    typed() {
        tab_call "$tab" evaluate '{"expression": "document.getElementById(\"name\").value"}' "$ui"
        [[ $(jq -r .result <<<"$BODY") == a ]]
    }
    wait_until typed 10

    # A dialog opens in the viewer and closes with the answer.
    click_at '#ask'
    wait_until 'ws_has "$out" ".type == \"dialog\" and .message == \"hello\""' 10
    ws_send '{"type":"dialog_answer","accept":true}'
    wait_until 'grep -q "\"type\":\"dialog_closed\"" "$out"' 10
    wait_until "title_is asked" 10

    # Hand back: the original Client holds the Tab again.
    rest POST "/v1/tabs/$tab/hand-back" '{}' "$ui"
    assert_status 200 "hand back"
    assert_json ".lease.client_id == \"$agent\"" "hand-back returns the Lease to the agent"
    rest GET "/v1/tabs/$tab"
    assert_json ".lease.client_id == \"$agent\"" "tab.get shows the agent as holder"

    # Closing the Tab ends the socket with a closed message.
    rest DELETE "/v1/tabs/$tab" '' "$agent"
    assert_status 200 "close the tab"
    wait_until 'grep -q "\"type\":\"closed\"" "$out"' 10
    ws_close
    wait_until '[[ $(instance_state "$instance") == stopped ]]' 15
    wait_until 'no_marker_pids "$instance"' 15
}

new_profile eph ephemeral
live_view_checks chromium eph

# A Client that does not exist is refused before the upgrade.
launch eph "$agent" "$base"
instance=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
ws_probe "/v1/tabs/$tab/screencast?client_id=$(ulid)"
assert_status 404 "unknown client_id before the upgrade"
stop_instance "$instance"
wait_until 'no_marker_pids "$instance"' 15

# Camoufox frames arrive too.
if have_camoufox; then
    seed_camoufox
    rest POST /v1/clients '{"label": "agent-1"}'
    agent=$(jqv '.id')
    rest POST /v1/clients '{"kind": "ui", "label": "ui"}'
    ui=$(jqv '.id')
    new_profile fox ephemeral camoufox-headless
    launch fox "$agent" "$base"
    instance=$(jqv '.instance.id')
    tab=$(jqv '.tab.id')
    out=$SCRATCH/camoufox-ws.log
    ws_client "ws://$BIND/v1/tabs/$tab/screencast?client_id=$ui" "$out" 60
    wait_until '[[ $(frames_in "$out") -ge 1 ]]' 10
    ws_close
    stop_instance "$instance"
    wait_until 'no_marker_pids "$instance"' 15
fi

finish "screencast frames, LC6 refusal, take over, input, dialog, hand back, closed, camoufox frames"
