#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail

rest GET /v1/settings
assert_status 200 settings.get
assert_json '.settings.ui.theme | type == "string"'

rest PATCH /v1/settings '{"ui.theme": "dark"}'
assert_status 200 "settings.set ui.theme"
assert_json '.settings.ui.theme == "dark"'
assert_json '.restart_required == []'

rest GET /v1/settings
assert_json '.settings.ui.theme == "dark"' "setting persisted in the daemon"

CHECKS=$((CHECKS + 1))
grep -q '^theme = "dark"' "$XDG_CONFIG_HOME/brr/config.toml" || fail "config.toml lacks theme = \"dark\""

rest GET '/v1/events?kinds=settings.changed'
assert_json '.events | map(select(.data.keys | index("ui.theme"))) | length == 1' "settings.changed event for ui.theme"

# The scratch daemon reads both of these from the environment, so the file must not shadow them.
rest PATCH /v1/settings '{"server.bind": "127.0.0.1:1"}'
assert_status 400 "key set by env"
assert_json '.code == "InvalidArgument" and .details.field == "server.bind"'
assert_json '.message | contains("BRR_SERVER__BIND")'

rest PATCH /v1/settings '{"vault.key_source": "keyring"}'
assert_status 400 "vault.key_source"
assert_json '.details.field == "vault.key_source"'

rest PATCH /v1/settings '{"ui.nope": 1}'
assert_status 400 "unknown key"

CHECKS=$((CHECKS + 1))
! grep -q 'bind' "$XDG_CONFIG_HOME/brr/config.toml" || fail "rejected keys reached config.toml"

out=$(brr --json settings get)
CHECKS=$((CHECKS + 1))
jq -e '.settings.ui.theme == "dark"' >/dev/null <<<"$out" || fail "CLI settings get disagrees"

finish "settings get/set, env-pinned keys rejected"
