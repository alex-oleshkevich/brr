#!/usr/bin/env bash
# M6: login checks: failed at launch, run with auto_relogin, Profile login_status, a stopped Profile.
# env: BRR_RETENTION__FREE_DISK_FLOOR_GB=1
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

PASSWORD=S3cret-Live-9f2
TOTP=JBSWY3DPEHPK3PXP

detect_builds
site_port=$(free_port)
start_fixture site "$site_port"
base=http://127.0.0.1:$site_port

new_profile checked ephemeral
profile_id=$(jqv '.id')
rest POST /v1/credentials "$(jq -nc --arg site "$base" --arg url "$base/login" --arg p "$PASSWORD" --arg t "$TOTP" \
    '{site: $site, login_url: $url, username: "u1", password: $p, totp: $t, profile_ids: ["checked"]}')"
assert_status 200 credential.create
credential=$(jqv '.id')

# Both expectations at once, and a missing site, are refused.
rest POST /v1/profiles/checked/login-checks "$(jq -nc --arg site "$base" --arg url "$base/account" \
    '{site: $site, url: $url, expect_text: "Logged in as", expect_selector: "p"}')"
assert_status 400 "two expectations"
assert_json '.code == "InvalidArgument"'

rest POST /v1/profiles/checked/login-checks "$(jq -nc --arg site "$base" --arg url "$base/account" --arg c "$credential" \
    '{site: $site, url: $url, expect_text: "Logged in as", credential_id: $c}')"
assert_status 200 login_check.create
assert_json '.last_status == "unknown" and .auto_relogin == false'
check=$(jqv '.id')

check_results() { # prints the statuses of the login.check_result events, oldest first
    rest GET "/v1/events?kinds=login.check_result&order=asc&limit=100"
    jq -r '[.events[].data.status] | join(",")' <<<"$BODY"
}

launch checked "" "$base/account"
instance=$(jqv '.instance.id')
wait_until '[[ $(check_results) == failed ]]' 60

rest PATCH "/v1/login-checks/$check" '{"auto_relogin": true}'
assert_status 200 login_check.update
assert_json '.auto_relogin == true'

rest POST "/v1/login-checks/$check/run" '{}'
assert_status 200 login_check.run
assert_json '.last_status == "ok"' "auto_relogin logged in and the check passed"
assert_json '.last_checked_at != null'

rest GET /v1/profiles
assert_status 200 profile.list
CHECKS=$((CHECKS + 1))
jq -e --arg id "$profile_id" '.profiles[] | select(.id == $id) | .login_status == "ok"' >/dev/null <<<"$BODY" ||
    fail "profile.list login_status is not ok"

stop_instance "$instance"
rest POST "/v1/login-checks/$check/run" '{}'
assert_status 409 "run on a stopped Profile"
assert_json '.code == "Conflict" and .details.reason == "profile_not_running"'

rest DELETE "/v1/login-checks/$check"
assert_status 200 login_check.delete
rest POST "/v1/login-checks/$check/run" '{}'
assert_status 404 "run after delete"

finish "login check failed at launch, auto_relogin turned it ok, stopped Profile refused"
