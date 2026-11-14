#!/usr/bin/env bash
# §14: one typed error, shown consistently by REST, CLI and MCP.
source "$(dirname "$0")/lib.sh"
set -euo pipefail

rest GET /v1/profiles/nope
assert_status 404 "missing profile"
assert_json '.code == "NotFound"'
assert_json '.message | length > 0'

rest POST /v1/profiles '{"nonsense": 1}'
assert_status 400 "bad body"
assert_json '.code == "InvalidArgument"'
assert_json '.details.field | length > 0' "InvalidArgument names the field"

rest POST /v1/profiles '{"name": "bad name", "template": "chromium-headless"}'
assert_status 400 "bad profile name"
assert_json '.details.field == "name"'

rc=0
out=$(brr profile get nope 2>/dev/null) || rc=$?
assert_eq "$rc" 3 "CLI NotFound exit code"
assert_eq "$out" "" "CLI stdout stays empty on error"

rc=0
brr profile create 'bad\ name' --template chromium-headless >/dev/null 2>&1 || rc=$?
assert_eq "$rc" 2 "CLI InvalidArgument exit code"

mcp_tool profile_get '{"profile": "nope"}'
assert_eq "$MCP_ERROR" true "MCP isError"
CHECKS=$((CHECKS + 1))
[[ $MCP_TEXT == "NotFound: "* ]] || fail "MCP error text starts with '<code>: ': $MCP_TEXT"
assert_json '.code == "NotFound"' "MCP body line carries the error JSON"
CHECKS=$((CHECKS + 1))
jq -e '.result | has("structuredContent") | not' >/dev/null <<<"$MCP_RAW" || fail "MCP tool errors carry no structuredContent"

assert_log 'op="profile.get"'
assert_log 'code="NotFound"'

stop_daemon
rc=0
brr daemon status >/dev/null 2>&1 || rc=$?
assert_eq "$rc" 69 "daemon unreachable exit code"

finish "REST, CLI and MCP agree on typed errors"
