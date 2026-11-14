#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
new_profile files persistent
peer_port=$(free_port)
peer_server "$peer_port"
launch files "" "http://127.0.0.1:$peer_port/set"
inst=$(jqv '.instance.id')
wait_title "$(jqv '.tab.id')" "cookie-set" 30

# Without -o a text output prints to stdout.
text=$(brr profile cookies export files --format netscape --instance-id "$inst")
CHECKS=$((CHECKS + 1))
[[ $text == *brr*online* ]] || fail "cookies.txt on stdout lacks the brr cookie: $text"
stop_instance "$inst"
wait_until '[[ $(instance_state "$inst") == stopped ]]' 15

# With -o the file is written and stdout stays empty.
html='<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF="https://example.com/a">A</A></DL>'
printf '%s' "$html" >"$SCRATCH/in.html"
brr profile bookmarks import files -i "$SCRATCH/in.html" >/dev/null
stdout=$(brr profile bookmarks export files -o "$SCRATCH/out.html" 2>/dev/null)
assert_eq "$stdout" "" "stdout with -o"
CHECKS=$((CHECKS + 1))
grep -q 'https://example.com/a' "$SCRATCH/out.html" || fail "out.html lacks the bookmark"

# A file over 48 MB is refused by the CLI before any request.
truncate -s 49M "$SCRATCH/big.txt"
before=$(grep -c 'op="profile.cookies_import"' "$XDG_STATE_HOME/brr/daemon.log" || true)
rc=0
err=$(brr profile cookies import files -i "$SCRATCH/big.txt" 2>&1 >/dev/null) || rc=$?
CHECKS=$((CHECKS + 1))
((rc != 0)) || fail "49 MB upload succeeded"
[[ $err == *"limit is 48 MB"* ]] || fail "error does not name the limit: $err"
after=$(grep -c 'op="profile.cookies_import"' "$XDG_STATE_HOME/brr/daemon.log" || true)
assert_eq "$after" "$before" "the daemon saw a request for the oversized file"

finish "text export on stdout, -o file, 48 MB cap before any request"
