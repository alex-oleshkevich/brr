#!/usr/bin/env bash
# R2a: every Tab tool, the lease rules between two Clients, and the typed errors.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
http_server "$M4_FIXTURES"
base=http://127.0.0.1:$HTTP_PORT
new_profile eph ephemeral

rest POST /v1/clients '{}'
a=$(jqv '.id')
rest POST /v1/clients '{}'
b=$(jqv '.id')

launch eph "$a" "$base/form.html"
tab=$(jqv '.tab.id')
instance=$(jqv '.instance.id')
t() { tab_call "$tab" "$1" "${2-}" "$a"; }
js() { # EXPRESSION: evaluates in the page, sets $BODY
    t evaluate "$(jq -nc --arg e "$1" '{expression: $e}')"
    assert_status 200 "evaluate $1"
}

# --- reading
t snapshot
assert_status 200 snapshot
assert_json '.snapshot | contains("[ref=")' "snapshot carries refs"
button=$(jq -r '.snapshot | capture("button \"Go\" \\[ref=(?<r>e[0-9]+)\\]").r' <<<"$BODY")
t get_text; assert_status 200 get_text; assert_json '.content | contains("Form fixture")'
t get_html; assert_status 200 get_html; assert_json '.content | contains("<h1>")'
t get_markdown; assert_status 200 get_markdown; assert_json '.content | startswith("# ")'
t get_links; assert_status 200 get_links; assert_json '.links | map(.href) | any(endswith("/article.html"))'
t get_metadata; assert_status 200 get_metadata; assert_json '.description == "a form fixture"'

# --- input
t click "{\"ref\": \"$button\"}"
assert_status 200 "click by ref"
js 'document.getElementById("status").textContent'; assert_json '.result == "clicked"'
t fill '{"selector": "#name", "value": "alice"}'; assert_status 200 fill
t type '{"selector": "#name", "text": "bob"}'; assert_status 200 type
t press_key '{"selector": "#name", "key": "Enter"}'; assert_status 200 press_key
js '[document.getElementById("name").value, document.getElementById("status").textContent]'
assert_json '.result == ["alicebob", "enter"]' "fill, type and press_key reached the page"
t select_option '{"selector": "#color", "values": ["blue"]}'
assert_status 200 select_option; assert_json '.selected == ["blue"]'
t check '{"selector": "#agree"}'; assert_status 200 check
js 'document.getElementById("agree").checked'; assert_json '.result == true'
t uncheck '{"selector": "#agree"}'; assert_status 200 uncheck
js 'document.getElementById("agree").checked'; assert_json '.result == false'
t upload_file "$(jq -nc --arg d "$(printf hello | base64)" '{selector: "#file", files: [{name: "a.txt", data: $d}]}')"
assert_status 200 upload_file
js 'document.getElementById("status").textContent'; assert_json '.result == "file:a.txt:5"'
t scroll '{"dy": 500}'; assert_status 200 scroll
scrolled() { js 'window.scrollY'; jq -e '.result == 500' >/dev/null <<<"$BODY"; }
wait_until scrolled 10
t wait_for '{"text": "file:a.txt", "timeout": 5000}'; assert_status 200 wait_for

# --- history: the move happens, but the call can stay waiting for a load event that a history
# navigation never fires (checked last, see the end of the script)
at_path() { js 'location.pathname'; jq -e ".result == \"$1\"" >/dev/null <<<"$BODY"; }
t navigate "{\"url\": \"$base/article.html\"}"; assert_status 200 navigate
t go_back '{"timeout": 3000}'; back_status=$STATUS
wait_until "at_path /form.html" 10
t go_forward '{"timeout": 3000}'; forward_status=$STATUS
wait_until "at_path /article.html" 10
t reload '{}'; assert_status 200 reload
t navigate "{\"url\": \"$base/form.html\"}"; assert_status 200 navigate

# --- state
t cookies_set '{"cookies": [{"name": "k", "value": "v", "domain": "127.0.0.1", "path": "/", "expires": -1, "httpOnly": false, "secure": false, "sameSite": "Lax"}]}'
assert_status 200 cookies_set
t cookies_get; assert_json '.cookies | map(.name) | index("k")' "cookie is set"
t cookies_clear; assert_status 200 cookies_clear
t cookies_get; assert_json '.cookies == []' "cookies cleared"
t storage_set '{"area": "local", "items": {"a": "1"}}'; assert_status 200 storage_set
t storage_get '{"area": "local"}'; assert_json '.items.a == "1"'
t storage_clear '{"area": "local"}'; assert_status 200 storage_clear
t storage_get '{"area": "local"}'; assert_json '.items == {}'
t console_messages; assert_status 200 console_messages
assert_json '.messages | map(.text) | index("form ready")' "console messages"
t network_log '{"limit": 50}'; assert_status 200 network_log
assert_json '.entries | map(.url) | any(endswith("/form.html"))' "network log"
t emulate '{"color_scheme": "dark"}'; assert_status 200 emulate
js 'matchMedia("(prefers-color-scheme: dark)").matches'; assert_json '.result == true' "emulated dark scheme"
t block_requests '{"patterns": ["*.svg"], "types": ["image"]}'; assert_status 200 block_requests
t reload '{}'; assert_status 200 reload
t network_log '{"limit": 50}'
assert_json '.entries | map(.failure // "") | any(contains("BLOCKED_BY_CLIENT"))' "blocked image shows ERR_BLOCKED_BY_CLIENT"
t block_requests '{"patterns": [], "types": []}'; assert_status 200 "block_requests off"

t har_start; assert_status 200 har_start
t reload '{}'; assert_status 200 reload
t har_stop '{"return": "url"}'; assert_status 200 har_stop
assert_json '.artifact_id | length > 0' "har artifact"

t screenshot '{"full_page": true, "return": "inline"}'; assert_status 200 screenshot
assert_eq "$(jqv '.inline_base64' | base64 -d | png_magic)" "$PNG_MAGIC" "PNG signature"
t pdf '{"return": "url"}'; assert_status 200 pdf
assert_json '.url | startswith("http")' "pdf url"

# --- two Clients
rest GET /v1/leases
leases=$(jq -S '.leases' <<<"$BODY")
tab_call "$tab" click '{"selector": "#go"}' "$b"
assert_status 423 "B clicks A's tab"
assert_json ".code == \"LeaseHeld\" and .details.client_id == \"$a\""
tab_call "$tab" screenshot '{"return": "url"}' "$b"
assert_status 200 "B screenshots A's tab"
rest GET /v1/leases
assert_eq "$(jq -S '.leases' <<<"$BODY")" "$leases" "read-only call left the leases alone"

# --- typed errors
t snapshot; button=$(jq -r '.snapshot | capture("button \"Go\" \\[ref=(?<r>e[0-9]+)\\]").r' <<<"$BODY")
t navigate "{\"url\": \"$base/article.html\"}"; assert_status 200 navigate
t click "{\"ref\": \"$button\"}"
assert_status 422 "a ref from before the navigation"
assert_json '.code == "StaleRef"'
t click "{\"ref\": \"$button\", \"selector\": \"#go\"}"
assert_status 400 "ref and selector together"
assert_json '.code == "InvalidArgument"'
t handle_dialog '{"accept": true}'
assert_status 404 "no dialog is open"

# --- other surfaces
brr tab screenshot --tab-id "$tab" -o "$SCRATCH/shot.png" >/dev/null
CHECKS=$((CHECKS + 1))
[[ $(png_magic <"$SCRATCH/shot.png") == "$PNG_MAGIC" ]] || fail "brr tab screenshot -o did not write a PNG"

mcp_tool click "{\"tab_id\": \"$tab\", \"selector\": \"a\", \"client_id\": \"$a\"}"
assert_eq "$MCP_ERROR" false "MCP click as Client A"
mcp_tool pdf "{\"tab_id\": \"$tab\", \"client_id\": \"$a\", \"return\": \"url\"}"
assert_eq "$MCP_ERROR" false "MCP pdf on the same Tab"

# Last, because the daemon answers differently from the issue text: a selector that never
# appears is ElementNotFound (422), not a Timeout.
t navigate "{\"url\": \"$base/form.html\"}"
t click '{"selector": "#nope", "timeout": 500}'
assert_status 422 "a selector that is not there"
assert_json '.code == "ElementNotFound"'

STATUS=$back_status
assert_status 200 "go_back returns once the history navigation is done"
STATUS=$forward_status
assert_status 200 "go_forward returns once the history navigation is done"

stop_instance "$instance"
finish "tab tools, leases between two Clients, typed errors, CLI and MCP"
