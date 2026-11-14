#!/usr/bin/env bash
# §7.2: promote keeps an ephemeral Instance's data as a new persistent Profile.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
port=$(free_port)
cookie_server "$port"
new_profile eph ephemeral

launch eph "" "http://127.0.0.1:$port/set"
instance=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
rest GET "/v1/tabs/$tab"
assert_json '.title == "set"' "the cookie was set"

rest POST "/v1/instances/$instance/promote" '{"name": "promoted"}'
assert_status 200 instance.promote

# Errors: the name is taken, the Instance is not ephemeral... and once stopped it cannot be promoted.
rest POST "/v1/instances/$instance/promote" '{"name": "eph"}'
assert_status 409 "promote onto an existing name"
assert_json '.code == "Conflict" and .details.reason == "name_taken"'

stop_instance "$instance"
wait_until '[[ $(instance_state "$instance") == stopped ]]' 15
rest POST "/v1/instances/$instance/promote" '{"name": "late"}'
assert_status 409 "promote a stopped Instance"
assert_json '.code == "InvalidTransition"'

rest GET /v1/profiles/promoted
assert_status 200 "promoted Profile exists"
assert_json '.kind == "persistent"' "promoted Profile is persistent"

launch promoted "" "http://127.0.0.1:$port/echo"
kept=$(jqv '.instance.id')
kept_tab=$(jqv '.tab.id')
rest GET "/v1/tabs/$kept_tab"
assert_json '.title == "cookie:live"' "the cookie survived the promote"

rest POST "/v1/instances/$kept/promote" '{"name": "again"}'
assert_status 400 "promote a persistent Instance"
assert_json '.code == "InvalidArgument"'
stop_instance "$kept"

finish "ephemeral Instance promoted to a persistent Profile with its cookie"
