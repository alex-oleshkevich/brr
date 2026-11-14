#!/usr/bin/env bash
# V3: a Live browser attached by install name, from the DevToolsActivePort file Chrome writes.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome
start_test_chrome

# XDG_CONFIG_HOME is the scratch dir: only files this script writes can be found.
mkdir -p "$XDG_CONFIG_HOME/chromium"
cp "$SCRATCH/chrome/DevToolsActivePort" "$XDG_CONFIG_HOME/chromium/DevToolsActivePort"

rest POST /v1/instances/attach '{"install_name": "chromium"}'
assert_status 200 "attach by install name"
first=$(jqv '.instance.id')
assert_json '.instance.kind == "live" and .instance.state == "attached"'
rest POST "/v1/instances/$first/detach" '{}'
assert_status 200 detach

# The CLI takes the same name.
out=$(brr instance attach --install-name chromium --json)
CHECKS=$((CHECKS + 1))
second=$(jq -er .instance.id <<<"$out") || fail "CLI attach printed no instance: $out"
[[ $second != "$first" ]] || fail "re-attach reused the Instance id"
kill -0 "$TEST_CHROME_PID" || fail "the test Chrome died"

rest POST /v1/instances/attach '{"install_name": "netscape"}'
assert_status 400 "unknown install name"
assert_json '.code == "InvalidArgument" and .details.field == "install_name"'

rest POST /v1/instances/attach '{"install_name": "chrome-beta"}'
assert_status 503 "no DevToolsActivePort for chrome-beta"
assert_json '.code == "DependencyMissing" and (.message | contains("chrome://inspect"))'

finish "install name via REST and CLI, unknown name 400, missing file 503 with the chrome://inspect hint"
