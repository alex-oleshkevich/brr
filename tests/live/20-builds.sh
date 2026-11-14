#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail

# Offline: source and channel validation, system Builds stay put, the doctor knows xz.
rest POST /v1/builds '{"source": "brave", "channel": "beta"}'
assert_status 400 "brave has no beta channel"
assert_json '.code == "InvalidArgument" and .details.field == "channel"'

rest GET /v1/system/doctor
assert_status 200 system.doctor
assert_json '[.checks[].name] | index("xz")' "doctor lists the xz check"

if have_chrome; then
    detect_builds
    system_id=$(jqv '[.builds[] | select(.source == "system")][0].id')
    rest DELETE "/v1/builds/$system_id"
    assert_status 400 "system Builds are not removable"
    assert_json '.code == "InvalidArgument" and .details.field == "build_id"'
    detected="system Build kept"
else
    detected="system remove skipped (no browser on PATH)"
fi

if [[ ${BRR_LIVE_NETWORK-} != 1 ]]; then
    finish "validation, doctor xz, $detected; network part skipped (BRR_LIVE_NETWORK unset)"
    exit 0
fi

# With network: headless_shell lifecycle and the other sources' version lists.
rest POST /v1/builds '{"source": "headless_shell", "channel": "stable"}'
assert_status 200 "install headless_shell stable"
shell_id=$(jqv '.id')
wait_until 'rest GET "/v1/builds/$shell_id"; [[ $(jq -r .state <<<"$BODY") == installed ]]' 600
assert_json '.capabilities | index("unpacked_extensions") | not' "headless_shell lacks unpacked_extensions"

sources=(camoufox ungoogled fingerprint_chromium brave)
[[ $(uname -m) == x86_64 ]] && sources+=(edge)
for source in "${sources[@]}"; do
    rest GET "/v1/builds/available?source=$source"
    assert_status 200 "available $source"
    assert_json '.available | length >= 1' "$source lists a version"
done

new_profile pinned persistent
profile_id=$(jqv '.id')
profile_set "$profile_id" '.build = {build_id: $b}' --arg b "$shell_id"

rest DELETE "/v1/builds/$shell_id"
assert_status 409 "a referenced Build is not removable"
assert_json '.code == "Conflict" and .details.reason == "referenced"'
rest GET /v1/builds
assert_json "[.builds[] | select(.id == \"$shell_id\") | .referenced_by[].id] | index(\"$profile_id\")" "build.list shows referenced_by"

rest GET "/v1/builds/$shell_id"
shell_path=$(jqv '.path')
profile_set "$profile_id" '.build = {channel: "cft:stable"}'

rest DELETE "/v1/builds/$shell_id"
assert_status 200 "remove the unreferenced Build"
wait_until '[[ ! -e $shell_path ]]' 10

finish "validation, doctor xz, $detected, headless_shell install/pin/remove, source lists"
