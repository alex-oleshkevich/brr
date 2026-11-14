#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
new_profile flags persistent
profile_set flags '.webrtc = "disable_non_proxied_udp" | .prefs = {"intl.accept_languages": "de-DE"}'

launch flags
inst=$(jqv '.instance.id')
data_dir=$(jqv '.instance.data_dir')

pid=$(jqv '.instance.pid')
# Chrome rewrites its argv in place, so the arguments may be separated by spaces rather than NULs.
cmdline=$(tr '\0' ' ' <"/proc/$pid/cmdline")
CHECKS=$((CHECKS + 1))
[[ " $cmdline " == *" --force-webrtc-ip-handling-policy=disable_non_proxied_udp "* ]] ||
    fail "browser $pid lacks --force-webrtc-ip-handling-policy=disable_non_proxied_udp"

stop_instance "$inst"
wait_until '[[ $(instance_state "$inst") == stopped ]]' 15
prefs=$data_dir/Default/Preferences
CHECKS=$((CHECKS + 1))
[[ -f $prefs ]] || fail "no Preferences file at $prefs"
BODY=$(<"$prefs")
assert_json '.intl.accept_languages == "de-DE"' "the pref reached Preferences"

finish "webrtc policy flag on the command line, prefs written"
