#!/usr/bin/env bash
# M5: a Camoufox Instance end to end: Tab tools, engine limits, a persistent Profile's cookie and identity.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_camoufox

seed_camoufox
http_server "$M4_FIXTURES"
base=http://127.0.0.1:$HTTP_PORT
new_profile fox ephemeral camoufox-headless

launch fox "" "$base/form.html"
instance=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
assert_eq "$(instance_state "$instance")" running "camoufox instance state"
t() { tab_call "$tab" "$1" "${2-}"; }
js() { # EXPRESSION: evaluates in the page, sets $BODY
    t evaluate "$(jq -nc --arg e "$1" '{expression: $e}')"
    assert_status 200 "evaluate $1"
}

# --- reading and input
t snapshot
assert_status 200 snapshot
assert_json '.snapshot | contains("[ref=")' "snapshot carries refs"
button=$(jq -r '.snapshot | capture("button \"Go\" \\[ref=(?<r>e[0-9]+)\\]").r' <<<"$BODY")
t click "{\"ref\": \"$button\"}"
assert_status 200 "click by ref"
js 'document.getElementById("status").textContent'
assert_json '.result == "clicked"' "the click reached the page"
t fill '{"selector": "#name", "value": "alice"}'
assert_status 200 fill
js 'document.getElementById("name").value'
assert_json '.result == "alice"' "fill reached the page"

# --- captures
t screenshot '{"return": "inline"}'
assert_status 200 "png screenshot"
assert_eq "$(jqv '.inline_base64' | base64 -d | png_magic)" "$PNG_MAGIC" "PNG signature"
t screenshot '{"return": "inline", "format": "webp"}'
assert_status 200 "webp screenshot"
assert_eq "$(jqv '.inline_base64' | base64 -d | head -c 4)" RIFF "WEBP container"

# --- state
expires=$(($(date +%s) + 86400))
t cookies_set "{\"cookies\": [{\"name\": \"k\", \"value\": \"v\", \"domain\": \"127.0.0.1\", \"path\": \"/\", \"expires\": $expires, \"httpOnly\": false, \"secure\": false, \"sameSite\": \"Lax\"}]}"
assert_status 200 cookies_set
t cookies_get
assert_json '.cookies | map(.name) | index("k")' "cookie is set"

t block_requests '{"patterns": ["*.svg"], "types": ["image"]}'
assert_status 200 block_requests
t reload '{}'
assert_status 200 reload
t network_log '{"limit": 50}'
assert_status 200 network_log
assert_json '.entries | map(select(.url | contains("pic.svg")) | .failure // "") | any(length > 0)' "the blocked image failed"
t block_requests '{"patterns": [], "types": []}'
assert_status 200 "block_requests off"

js 'navigator.userAgent'
assert_json '.result | contains("Firefox")' "user agent says Firefox"

# --- what Camoufox cannot do
t pdf '{"return": "url"}'
assert_status 422 "pdf on camoufox"
assert_json '.code == "EngineUnsupported" and .details.capability == "pdf"'
ws_probe "/cdp?profile=fox"
assert_status 400 "/cdp on a camoufox profile"

# --- stop
stop_instance "$instance"
wait_until '[[ $(instance_state "$instance") == stopped ]]' 15
wait_until 'no_marker_pids "$instance"' 15

# --- a persistent Profile keeps its cookie and its identity
new_profile keep persistent camoufox-headless
launch keep "" "$base/form.html"
keep_instance=$(jqv '.instance.id')
keep_tab=$(jqv '.tab.id')
tab=$keep_tab
js 'navigator.userAgent'
first_ua=$(jqv '.result')
t cookies_set "{\"cookies\": [{\"name\": \"kept\", \"value\": \"yes\", \"domain\": \"127.0.0.1\", \"path\": \"/\", \"expires\": $expires, \"httpOnly\": false, \"secure\": false, \"sameSite\": \"Lax\"}]}"
assert_status 200 "cookies_set on the persistent profile"
stop_instance "$keep_instance"
wait_until '[[ $(instance_state "$keep_instance") == stopped ]]' 15
wait_until 'no_marker_pids "$keep_instance"' 15

rest GET "/v1/profiles/keep/cookies?format=json"
assert_status 200 "cookies export of the stopped profile"
assert_json '.data | contains("kept")' "the cookie survived the stop"

launch keep "" "$base/form.html"
keep_instance=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
js 'navigator.userAgent'
assert_eq "$(jqv '.result')" "$first_ua" "the same identity on the next launch"
stop_instance "$keep_instance"
wait_until '[[ $(instance_state "$keep_instance") == stopped ]]' 15
wait_until 'no_marker_pids "$keep_instance"' 15

finish "camoufox ephemeral instance, tab tools, engine limits, persistent cookie and identity"
