#!/usr/bin/env bash
# M5 R3: the real Camoufox install: assets, progress, the compatibility suite and its consequence for latest-compatible.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
[[ -n ${BRR_LIVE_NETWORK-} ]] || skip "BRR_LIVE_NETWORK is not set (the install downloads about 2.2 GB)"

sse=$SCRATCH/sse.txt
curl -sN --unix-socket "$SOCK" -H 'Last-Event-ID: 0' http://localhost/v1/events/stream >"$sse" &
SLOW_PIDS+=($!)
rest POST /v1/builds '{"source": "camoufox", "channel": "latest-compatible"}'
assert_status 200 "build.install camoufox"
build_id=$(jqv '.id')

# The install task ends with build.installed (suite passed), or build.install_failed (it did not, or an asset failed).
finished() {
    rest GET "/v1/events?kinds=build.installed,build.install_failed&order=asc&limit=100"
    jq -e --arg id "$build_id" '[.events[] | select(.data.build_id == $id)] | length > 0' >/dev/null <<<"$BODY"
}
wait_until finished 850

CHECKS=$((CHECKS + 1))
grep -q '^event: build.install_progress' "$sse" || fail "no build.install_progress on SSE"

rest GET "/v1/events?kinds=build.installed,build.install_failed&order=asc&limit=100"
installed=$(jq -r --arg id "$build_id" '[.events[] | select(.data.build_id == $id and .kind == "build.installed")] | length' <<<"$BODY")
failed=$(jq -r --arg id "$build_id" '[.events[] | select(.data.build_id == $id and .kind == "build.install_failed")] | length' <<<"$BODY")
error=$(jq -r --arg id "$build_id" '[.events[] | select(.data.build_id == $id and .kind == "build.install_failed")][0].data.error // ""' <<<"$BODY")
rest GET "/v1/events?kinds=build.channel_moved&order=asc&limit=100"
moved=$(jq -r '[.events[] | select(.data.source == "camoufox")] | length' <<<"$BODY")
rest GET "/v1/builds/$build_id"
assert_status 200 "build.get"
build_dir=$(dirname "$(jqv '.path')")
channel=$(jq -r '.channel // "none"' <<<"$BODY")
compat=$build_dir/brr-compat.json

assets=$XDG_DATA_HOME/brr/assets
if [[ $error != *"compatibility suite failed"* && $failed == 1 && $installed == 0 ]]; then
    # An asset install failed: nothing more is asserted about the suite.
    fail "install failed before the suite ran: $error"
fi
CHECKS=$((CHECKS + 1))
compgen -G "$assets/fonts-bundle-*/groups.json" >/dev/null || fail "no fonts bundle under $assets"
compgen -G "$assets/fpgen-model/*/values.dat" >/dev/null || fail "no fpgen model under $assets"
compgen -G "$assets/geoip/*.mmdb" >/dev/null || fail "no GeoIP database under $assets"
[[ -f $compat ]] || fail "no compatibility record at $compat"

if [[ $installed == 1 ]]; then
    assert_eq "$(jq -r .passed "$compat")" true "record says the suite passed"
    assert_eq "$failed" 0 "no install_failed after a pass"
    assert_eq "$channel" latest-compatible "the Build is on the channel"
    assert_eq "$moved" 1 "channel_moved published once"
else
    assert_eq "$(jq -r .passed "$compat")" false "record says the suite failed"
    assert_eq "$failed" 1 "install_failed after a failure"
    assert_eq "$(jq -r .failed_step "$compat")" "$(sed -n 's/^compatibility suite failed at \([a-z]*\).*/\1/p' <<<"$error")" "install_failed names the failed step"
    assert_eq "$channel" none "a failing Build is not on the channel"
    assert_eq "$moved" 0 "no channel_moved after a failure"
fi

finish "camoufox install: $([[ $installed == 1 ]] && echo 'suite passed, channel moved' || echo "suite failed (${error:0:80}), no channel move")"
