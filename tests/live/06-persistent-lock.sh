#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
new_profile pers persistent

launch pers
first=$(jqv '.instance.id')
assert_json '.instance.ephemeral == false'

rest POST /v1/profiles/pers/launch '{}'
assert_status 409 "second launch of a persistent profile"
assert_json '.code == "Conflict" and .details.reason == "profile_locked"'

stop_instance "$first"
wait_until '[[ $(instance_state "$first") == stopped ]]' 10

launch pers
second=$(jqv '.instance.id')
CHECKS=$((CHECKS + 1))
[[ $second != "$first" ]] || fail "relaunch reused instance id"
stop_instance "$second"

finish "persistent profile lock held while running, released on stop"
