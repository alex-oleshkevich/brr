#!/usr/bin/env bash
# §17.2: an MCP client that goes quiet loses its Lease, then its Instance is reaped. HTTP and stdio.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
set_settings '{"client.mcp_http_idle_min": 1, "idle.instance_min": 1, "idle.grace_s": 5}'

# HTTP: launch with a declared Client, then make no further calls.
mcp_tool client_create '{"kind": "mcp", "label": "expiry"}'
client=$(jqv '.id')
mcp_tool profile_create '{"name": "eph", "template": "chromium-headless", "kind": "ephemeral", "client_id": "'"$client"'"}'
mcp_tool profile_launch '{"profile": "eph", "client_id": "'"$client"'"}'
assert_eq "$MCP_ERROR" false profile_launch
http_instance=$(jqv '.instance.id')
rest GET "/v1/leases?client_id=$client"
assert_json '.leases | length >= 1' "the Client holds a Lease"

# stdio: a launch, then stdin closes.
in=$SCRATCH/mcp.in out=$SCRATCH/mcp.out
mkfifo "$in"
"$BRR_BIN" mcp <"$in" >"$out" 2>/dev/null &
mcp_pid=$!
exec 3>"$in"
send() { echo "$1" >&3; }
reply() {
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
exec 3>&-
no_mcp_stdio_lease() {
    rest GET /v1/leases
    jq -e "[.leases[] | select(.instance_id == \"$stdio_instance\")] | length == 0" >/dev/null <<<"$BODY"
}
wait_until no_mcp_stdio_lease 5
wait_until "! kill -0 $mcp_pid 2>/dev/null" 10

# HTTP Client expiry: the Lease is released and client.ended says why.
client_gone() {
    rest GET "/v1/leases?client_id=$client"
    jq -e '.leases == []' >/dev/null <<<"$BODY"
}
wait_until client_gone 200
rest GET '/v1/events?kinds=client.ended&limit=100'
assert_json "[.events[] | select(.data.client_id == \"$client\" and .data.reason == \"idle\")] | length == 1" "client.ended reason idle"

# Both Instances then walk idle, reaping, stopping, stopped.
for instance in "$http_instance" "$stdio_instance"; do
    wait_until '[[ $(instance_state "$instance") == stopped ]]' 300
    trail=$(state_trail "$instance" | tr '\n' ' ')
    CHECKS=$((CHECKS + 1))
    [[ $trail == *"idle:idle_timeout reaping:reap stopping:grace_over stopped:exited"* ]] || fail "unexpected trail for $instance: $trail"
done

finish "HTTP client idle expiry and stdio close release Leases; Instances reaped afterwards"
