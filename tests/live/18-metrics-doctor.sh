#!/usr/bin/env bash
# /metrics, /v1/system/doctor and `brr doctor` (which works without a daemon).
source "$(dirname "$0")/lib.sh"
set -euo pipefail

metrics() {
    curl -sS --max-time 30 --unix-socket "$SOCK" http://localhost/metrics
}

text=$(metrics)
CHECKS=$((CHECKS + 1))
grep -q '^brr_admission_queue_depth ' <<<"$text" || fail "no brr_admission_queue_depth in /metrics"

rest GET /v1/system/doctor
assert_status 200 system.doctor
assert_json '.checks | length >= 11' "at least the 11 checks of §15"
assert_json '[.checks[] | .status] | all(. == "ok" or . == "warn" or . == "fail")' "every check has a status"
assert_json '[.checks[] | select(.name == "chromium_build")] | length == 1 and .[0].status == "fail"' "no Build yet"

# No Build registered: brr doctor reads the machine itself and does not start a daemon.
stop_daemon
rm -f "$SOCK"
rc=0
doctor=$(brr doctor 2>&1) || rc=$?
assert_eq "$rc" 1 "brr doctor exit code with no Build"
CHECKS=$((CHECKS + 1))
[[ $doctor == *chromium_build* || $doctor == *Chromium* ]] || fail "brr doctor does not mention the Build: $doctor"
CHECKS=$((CHECKS + 1))
[[ ! -S $SOCK ]] || fail "brr doctor started a daemon"
start_daemon

if have_chrome; then
    detect_builds
    new_profile eph ephemeral
    launch eph
    instance=$(jqv '.instance.id')
    text=$(metrics)
    CHECKS=$((CHECKS + 1))
    grep -qF 'brr_instances{state="running",engine="chromium"} 1' <<<"$text" || fail "no running chromium instance gauge: $(grep brr_instances <<<"$text")"
    rss=$(sed -n "s/^brr_instance_rss_bytes{instance=\"$instance\"} //p" <<<"$text")
    CHECKS=$((CHECKS + 1))
    [[ ${rss:-0} -gt 0 ]] || fail "brr_instance_rss_bytes for $instance is '${rss-}'"
    stop_instance "$instance"
    stop_daemon
    rm -f "$SOCK"
    rc=0
    brr doctor >/dev/null 2>&1 || rc=$?
    assert_eq "$rc" 0 "brr doctor exit code with a Build registered"
    start_daemon
    browser="browser metrics and doctor with a Build ran"
else
    browser="browser part skipped (no browser on PATH)"
fi

finish "metrics and doctor; $browser"
