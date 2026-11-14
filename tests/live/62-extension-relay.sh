#!/usr/bin/env bash
# V3a, E3: the real companion extension relays a Chrome's shared tabs to the daemon.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

REPO=$(cd "$(dirname "$0")/../.." && pwd)
scripts_dir=$REPO/scripts
"$scripts_dir/ext-build.sh" >/dev/null 2>&1 || fail "just ext-build failed"
dist=$REPO/extension/dist
ext_id=$(extension_id "$dist")

# Chrome keeps --load-extension working for Chrome for Testing; newer builds also need the feature off.
start_test_chrome --load-extension="$dist" --disable-features=DisableLoadExtensionCommandLineSwitch
http_server "$M7_FIXTURES"
base=http://127.0.0.1:$HTTP_PORT
cdp="http://127.0.0.1:$TEST_CHROME_PORT"

# --- Live Instance A drives the user's Chrome: it opens a page and the extension popup
rest POST /v1/instances/attach "$(jq -nc --arg u "$cdp" '{cdp_url: $u}')"
assert_status 200 "attach A by cdp_url"
a_inst=$(jqv '.instance.id')

rest POST "/v1/instances/$a_inst/tabs" "$(jq -nc --arg u "$base/one.html" '{url: $u}')"
assert_status 200 "open the fixture page through A"
fixture_target=$(jqv '.tab.target_id')
# The popup is opened the way a user would, through Chrome itself; A does not list it, so the script
# drives it over its own page socket.
popup_ws=$(curl -fsS --max-time 10 -X PUT "$cdp/json/new?chrome-extension://$ext_id/popup.html" | jq -er .webSocketDebuggerUrl)
popup_js() { # EXPRESSION: evaluates in the popup, prints the value
    cdp_ws "$popup_ws" "$(jq -nc --arg e "$1" '[{method: "Runtime.evaluate", params: {expression: $e, returnByValue: true, awaitPromise: true}}]')" |
        jq -r 'select(.id == 1) | .result.result.value // empty'
}
popup_ready() {
    [[ $(popup_js 'document.querySelector("#allow") !== null && document.querySelector("#url").value !== ""') == true ]]
}
wait_until popup_ready 15
popup_js "const u = document.querySelector('#url'); u.value = 'ws://$BIND/v1/extension-relay'; u.dispatchEvent(new Event('input')); u.value" >/dev/null

(
    curl -sS --max-time 60 --unix-socket "$SOCK" -X POST -H 'content-type: application/json' \
        -d '{"via": "extension"}' -o "$SCRATCH/relay-attach.json" -w '%{http_code}' http://localhost/v1/instances/attach \
        >"$SCRATCH/relay-attach.status"
) &
attach_pid=$!
SLOW_PIDS+=($attach_pid)
sleep 1

popup_js 'document.querySelector("#allow").click(); true' >/dev/null
wait "$attach_pid" || fail "the via=extension attach call failed"
STATUS=$(<"$SCRATCH/relay-attach.status")
BODY=$(<"$SCRATCH/relay-attach.json")
assert_status 200 "instance.attach via the extension"
b_inst=$(jqv '.instance.id')
assert_json '.instance.kind == "live" and .instance.state == "attached"' "B is a Live Instance"
assert_json '(.tabs | length) == 1' "exactly one shared Tab"
assert_json '.tabs[0].url | endswith("one.html")' "the shared Tab is the fixture page"
b_tab=$(jqv '.tabs[0].id')

# --- commands round-trip through the real extension
tab_call "$b_tab" evaluate '{"expression": "document.title"}'
assert_status 200 "evaluate on B's Tab"
assert_json '.result == "fixture-one"'
rest POST "/v1/instances/$b_inst/tabs" "$(jq -nc --arg u "$base/two.html" '{url: $u}')"
assert_status 200 "tab.open on B"
new_tab=$(jqv '.tab.id')
rest GET "/v1/instances/$b_inst/tabs"
assert_json "[.tabs[] | select(.id == \"$new_tab\")] | length == 1" "the new Tab is in B's list"
rest POST "/v1/instances/$b_inst/stop" '{}'
assert_status 409 "stop on a relayed Live Instance"

# --- Disconnect from the popup ends B
popup_connected() {
    [[ $(popup_js '!document.querySelector("#connected").hidden') == true ]]
}
wait_until popup_connected 10
popup_js 'document.querySelector("#disconnect").click(); true' >/dev/null
wait_until "[[ \$(instance_state $b_inst) == lost ]]" 15

# --- E3: a Profile with the companion materializes it and the daemon serves the zip
detect_builds
new_profile comp persistent
profile_set comp '.companion = true'
launch comp
comp_inst=$(jqv '.instance.id')
data_dir=$(jqv '.instance.data_dir')
CHECKS=$((CHECKS + 1))
compgen -G "$XDG_DATA_HOME/brr/companion/*/brr.json" >/dev/null || fail "no companion/*/brr.json under $XDG_DATA_HOME/brr"
stop_instance "$comp_inst"
wait_until "[[ \$(instance_state $comp_inst) == stopped ]]" 15
# Chrome records the unpacked extension it loaded, by path, in the profile.
CHECKS=$((CHECKS + 1))
grep -rlF "$XDG_DATA_HOME/brr/companion/" "$data_dir/Default" >/dev/null 2>&1 || fail "the profile does not record the companion extension"
brr extension companion -o "$SCRATCH/c.zip" >/dev/null
listing=$(python3 -c 'import sys, zipfile; print("\n".join(zipfile.ZipFile(sys.argv[1]).namelist()))' "$SCRATCH/c.zip")
CHECKS=$((CHECKS + 1))
[[ $listing == *manifest.json* && $listing == *brr.json* ]] || fail "companion zip lacks manifest.json or brr.json: $listing"

# --- no connection: the attach times out after 30 s (slow)
if [[ -n ${BRR_LIVE_SLOW-} ]]; then
    rest POST /v1/instances/attach '{"via": "extension"}'
    assert_status 409 "attach via extension without a connection"
    assert_json '.code == "Conflict" and .details.reason == "extension_unanswered"'
fi

finish "relay attach shares only the fixture Tab, evaluate/tab.open round-trip, stop 409, Disconnect → lost, companion Profile and zip"
