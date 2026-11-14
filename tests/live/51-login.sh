#!/usr/bin/env bash
# M6: the login tool against a local three-step site with TOTP, and the secret-leak grep (§18).
# env: BRR_RETENTION__FREE_DISK_FLOOR_GB=1
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

PASSWORD=S3cret-Live-9f2
TOTP=JBSWY3DPEHPK3PXP
NOTES=note-secret-77

# Every response body this script sees is kept as raw bytes for the leak grep.
COLLECT=$SCRATCH/collected
mkdir -p "$COLLECT"
eval "$(declare -f rest | sed '1s/^rest/rest_plain/')"
eval "$(declare -f mcp_http | sed '1s/^mcp_http/mcp_http_plain/')"
rest() {
    rest_plain "$@"
    printf '%s %s %s\n%s\n' "$1" "$2" "$STATUS" "$BODY" >>"$COLLECT/rest.log"
}
mcp_http() {
    mcp_http_plain "$@"
    printf 'mcp %s\n%s\n' "$1" "$MCP_RAW" >>"$COLLECT/mcp.log"
}

detect_builds
site_port=$(free_port)
start_fixture site "$site_port"
base=http://127.0.0.1:$site_port

# The whole event stream of the script, byte for byte.
curl -sN --unix-socket "$SOCK" "http://localhost/v1/events/stream" >"$COLLECT/sse.log" &
SLOW_PIDS+=($!)

new_profile login-profile ephemeral
rest POST /v1/clients '{}'
client=$(jqv '.id')

credential() { # LOGIN_PATH [LINK]: creates a credential for the fixture site, prints its id
    local profiles='["login-profile"]' user=u1
    # One credential per site and username.
    [[ $1 == /login && ${2-link} == link ]] || user="u1-${1#/}-${2-link}"
    [[ ${2-link} == nolink ]] && profiles='[]'
    rest POST /v1/credentials "$(jq -nc --arg site "$base" --arg url "$base$1" --arg p "$PASSWORD" --arg t "$TOTP" \
        --arg n "$NOTES" --arg user "$user" --argjson profiles "$profiles" \
        '{site: $site, login_url: $url, username: $user, password: $p, totp: $t, notes: $n, profile_ids: $profiles}')"
    assert_status 200 credential.create
    jqv '.id'
}

good=$(credential /login)
wrong=$(credential /wrong)
sticky=$(credential /sticky)
unlinked=$(credential /login nolink)

# An update without a password keeps the stored one: the login below still works.
rest PATCH "/v1/credentials/$good" '{"username": "u1"}'
assert_status 200 "update without password"

launch login-profile "$client" "$base/account"
tab=$(jqv '.tab.id')
instance=$(jqv '.instance.id')
tab_ready() {
    rest GET "/v1/tabs/$tab"
    [[ $(jq -r .state <<<"$BODY") == ready ]]
}
wait_until tab_ready 20

# --- happy path through MCP
mcp_tool login "$(jq -nc --arg t "$tab" --arg c "$good" --arg cl "$client" '{tab_id: $t, credential_id: $c, client_id: $cl}')"
assert_eq "$MCP_ERROR" false "MCP login: $MCP_TEXT"
assert_json '.ok == true and .steps == 3' "three steps: username, password, code"
assert_json ".url | endswith(\"/account\")" "ended on the account page"
tab_call "$tab" get_text '{}' "$client"
assert_status 200 get_text
assert_json '.content | contains("Logged in as u1")' "the site accepted the login"

# --- errors
tab_call "$tab" login "$(jq -nc --arg c "$unlinked" '{credential_id: $c}')" "$client"
assert_status 409 "credential not linked"
assert_json '.code == "Conflict" and .details.reason == "credential_not_linked"'

tab_call "$tab" login "$(jq -nc --arg c "$wrong" '{credential_id: $c}')" "$client"
assert_status 422 "site rejects the password"
assert_json '.code == "LoginFailed" and .details.step == 2' "rejected at the password step"

# What a page still holds after the failed login: no password field value, no secret in the DOM.
tab_call "$tab" evaluate '{"expression": "document.querySelector(\"input[type=password]\").value"}' "$client"
assert_status 200 "evaluate the password field"
assert_json '.result == ""' "the password field is empty after the rejection"
tab_call "$tab" snapshot '{}' "$client"
assert_status 200 snapshot
tab_call "$tab" get_html '{}' "$client"
assert_status 200 get_html

# --- masking: a text-type field the login filled is a disc in any screenshot
tab_call "$tab" login "$(jq -nc --arg c "$sticky" '{credential_id: $c}')" "$client"
assert_status 200 "login on the page that keeps its fields"
tab_call "$tab" get_metadata '{}' "$client"
assert_status 200 get_metadata
assert_json '.title == "ts:disc"' "the filled code field was masked when the form was submitted"
tab_call "$tab" evaluate '{"expression": "1"}' "$client"
assert_status 409 "evaluate is refused while the page holds a filled secret"
assert_json '.details.reason == "secret_on_page"'
tab_call "$tab" screenshot '{}' "$client"
assert_status 200 screenshot

rest GET "/v1/events?limit=1000&order=asc"
assert_status 200 "events.list"
assert_json '[.events[] | select(.kind == "login.performed")] | length >= 3' "login.performed events"
wait_until "grep -q 'login.performed' '$COLLECT/sse.log'" 10
sleep 1

# --- the leak grep over raw bytes
cp "$XDG_STATE_HOME/brr/daemon.log" "$COLLECT/daemon.log"
bytes=$(cat "$COLLECT"/* | wc -c)
CHECKS=$((CHECKS + 1))
((bytes > 1000)) || fail "the leak grep collected only $bytes bytes"
for secret in "$PASSWORD" "$TOTP" "$NOTES"; do
    matches=$( (grep -rFc -- "$secret" "$COLLECT" || true) | awk -F: '{n += $NF} END {print n + 0}')
    assert_eq "$matches" 0 "leak: '$secret' found in $COLLECT"
done
echo "leak grep: searched $bytes bytes (responses, MCP results, events, SSE, snapshot, html, evaluate, daemon.log)"

stop_instance "$instance"
finish "login through MCP, typed errors, masking, no secret in $bytes bytes of output"
