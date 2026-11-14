#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail

mcp_http tools/list '{}'
assert_eq "$MCP_STATUS" 200 "tools/list"
CHECKS=$((CHECKS + 1))
jq -e '.result.tools | map(.name) | index("client_create") and index("profile_launch") and index("navigate")' >/dev/null <<<"$MCP_RAW" ||
    fail "tools/list lacks core tools"

# Any Host header is fine; the socket is the trust boundary (no 403).
host_status=$(curl -s -o /dev/null -w '%{http_code}' -X POST "http://$BIND/mcp" -H 'Host: example.com' \
    -H 'accept: application/json, text/event-stream' -H 'content-type: application/json' \
    --data '{"jsonrpc":"2.0","id":1,"method":"ping"}')
CHECKS=$((CHECKS + 1))
[[ $host_status != 403 ]] || fail "/mcp rejected Host: example.com with 403"

mcp_tool client_create '{"kind": "mcp", "label": "live"}'
assert_eq "$MCP_ERROR" false client_create
client=$(jqv '.id')

mcp_tool profile_create '{"name": "eph", "template": "chromium-headless", "kind": "ephemeral", "client_id": "'"$client"'"}'
assert_eq "$MCP_ERROR" false profile_create
assert_json '.name == "eph"'

mcp_tool profile_get '{"profile": "nope", "client_id": "'"$client"'"}'
assert_eq "$MCP_ERROR" true "error through a declared Client"
assert_json '.code == "NotFound"'

mcp_tool lease_list '{"client_id": "'"$client"'"}'
assert_json '.leases == []' "lease_list takes client_id as its own input"

mcp_tool profile_get "{\"profile\": \"eph\", \"client_id\": \"$(ulid)\"}"
assert_eq "$MCP_ERROR" true "unknown client_id"
CHECKS=$((CHECKS + 1))
[[ $MCP_TEXT == *client_create* ]] || fail "expired client error points at client_create: $MCP_TEXT"

if have_chrome; then
    detect_builds

    mcp_tool profile_launch '{"profile": "eph", "client_id": "'"$client"'", "url": "data:text/html,<title>m</title>"}'
    assert_eq "$MCP_ERROR" false profile_launch
    assert_json '.tab.state == "ready"'
    tab=$(jqv '.tab.id')
    instance=$(jqv '.instance.id')

    mcp_tool navigate '{"tab_id": "'"$tab"'", "url": "data:text/html,<title>n</title>", "client_id": "'"$client"'"}'
    assert_eq "$MCP_ERROR" false navigate
    rest GET "/v1/tabs/$tab"
    assert_json '.title == "n"' "navigate changed the page"

    rest GET "/v1/leases?client_id=$client"
    assert_json '.leases | length >= 1' "the declared Client holds its Lease across calls"
    mcp_tool client_end '{"client_id": "'"$client"'"}'
    assert_eq "$MCP_ERROR" false client_end
    rest GET "/v1/leases?client_id=$client"
    assert_json '.leases == []' "client_end released the Lease"
    stop_instance "$instance"

    # stdio: one Client per process; closing stdin ends it.
    in=$SCRATCH/mcp.in out=$SCRATCH/mcp.out
    mkfifo "$in"
    "$BRR_BIN" mcp <"$in" >"$out" 2>/dev/null &
    mcp_pid=$!
    exec 3>"$in"
    send() { echo "$1" >&3; }
    reply() { # ID: waits for the response with that id, leaves it in $BODY
        wait_until "jq -e 'select(.id == $1)' \"$out\" >/dev/null 2>&1" 20
        BODY=$(jq -c "select(.id == $1)" "$out")
    }
    send '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"live","version":"0"}}}'
    reply 1
    send '{"jsonrpc":"2.0","method":"notifications/initialized"}'
    send '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"profile_launch","arguments":{"profile":"eph"}}}'
    reply 2
    assert_json '.result.isError != true' "stdio profile_launch"
    stdio_instance=$(jq -r '.result.structuredContent.instance.id' <<<"$BODY")

    rest GET /v1/leases
    assert_json '[.leases[] | select(.client_kind == "mcp")] | length >= 1' "stdio Client holds a Lease"

    exec 3>&-
    no_mcp_lease() {
        rest GET /v1/leases
        jq -e '[.leases[] | select(.client_kind == "mcp")] | length == 0' >/dev/null <<<"$BODY"
    }
    wait_until no_mcp_lease 5
    wait_until "! kill -0 $mcp_pid 2>/dev/null" 10
    stop_instance "$stdio_instance"
    lease_part="lease and stdio parts ran"
else
    lease_part="lease part skipped (no browser on PATH)"
fi

finish "MCP HTTP tools and Host header; $lease_part"
