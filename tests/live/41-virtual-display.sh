#!/usr/bin/env bash
# M5: display virtual runs the browser on an Xvfb sized to the Profile's screen; no Xvfb is a typed error.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome
need_xvfb

# xvfb_pids ID: the Xvfb processes carrying the Instance's marker.
xvfb_pids() {
    local pid
    for pid in $(marker_pids "$1"); do
        [[ $(cat "/proc/$pid/comm" 2>/dev/null) == Xvfb ]] && echo "$pid"
    done
    return 0
}

detect_builds
new_profile screen ephemeral
profile_set screen '.display = "virtual" | .screen = {width: 1024, height: 768}'

launch screen
instance=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
tab_call "$tab" evaluate '{"expression": "screen.width + \"x\" + screen.height"}'
assert_status 200 "evaluate screen size"
assert_json '.result == "1024x768"' "screen matches the profile's screen"
CHECKS=$((CHECKS + 1))
[[ -n $(xvfb_pids "$instance") ]] || fail "no Xvfb process carries the Instance marker"
stop_instance "$instance"
wait_until 'no_marker_pids "$instance"' 15

# Without Xvfb on PATH the launch says what is missing.
stop_daemon 30 || fail "the daemon ignored SIGTERM"
DAEMON_PATH=$SCRATCH/bin start_daemon
rest POST /v1/profiles/screen/launch '{}'
assert_status 503 "launch without Xvfb"
assert_json '.code == "DependencyMissing" and .details.name == "Xvfb"'
stop_daemon 30 || fail "the daemon ignored SIGTERM"
start_daemon

# Camoufox on a virtual display, sized the same way.
if have_camoufox; then
    seed_camoufox
    new_profile stealth ephemeral camoufox-stealth
    profile_set stealth '.screen = {width: 1024, height: 768}'
    launch stealth
    stealth_instance=$(jqv '.instance.id')
    tab=$(jqv '.tab.id')
    tab_call "$tab" evaluate '{"expression": "screen.width + \"x\" + screen.height"}'
    assert_status 200 "evaluate camoufox screen size"
    assert_json '.result == "1024x768"' "camoufox screen matches the profile's screen"
    CHECKS=$((CHECKS + 1))
    [[ -n $(xvfb_pids "$stealth_instance") ]] || fail "no Xvfb process carries the camoufox Instance marker"
    stop_instance "$stealth_instance"
    wait_until 'no_marker_pids "$stealth_instance"' 15
fi

finish "virtual display sized to the screen, Xvfb marker, DependencyMissing without Xvfb, camoufox-stealth"
