#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
peer_port=$(free_port)
peer_server "$peer_port"

stop_and_wait() {
    stop_instance "$1"
    wait_until "[[ \$(instance_state $1) == stopped ]]" 15
}

# 1. The Profile's files exist after one run.
new_profile jar persistent
launch jar
stop_and_wait "$(jqv '.instance.id')"

# 2. Offline import of a Netscape cookie for the peer's host, through the CLI file flag.
expires=$(($(date +%s) + 3600))
printf '127.0.0.1\tFALSE\t/\tFALSE\t%s\tbrr\toffline\n' "$expires" >"$SCRATCH/cookies.txt"
BODY=$(brr profile cookies import jar -i "$SCRATCH/cookies.txt" --format netscape --json)
assert_json '.imported == 1' "cookie imported into the stopped profile"

# 3. The browser sends it; export from the running Instance sees it.
launch jar "" "http://127.0.0.1:$peer_port/echo"
inst=$(jqv '.instance.id')
wait_title "$(jqv '.tab.id')" "cookie:offline" 30
rest GET "/v1/profiles/jar/cookies?format=json&instance_id=$inst"
assert_status 200 "export from the running instance"
CHECKS=$((CHECKS + 1))
jq -e '.data | fromjson | .cookies | map(.name) | index("brr")' >/dev/null <<<"$BODY" || fail "storageState lacks the brr cookie"

# 4. Bookmarks: refused while running, imported and exported when stopped.
html='<!DOCTYPE NETSCAPE-Bookmark-file-1><DL><p><DT><A HREF="https://example.com/a">A</A><DT><A HREF="https://example.org/b">B</A></DL>'
printf '%s' "$html" >"$SCRATCH/bookmarks.html"
rest PUT /v1/profiles/jar/bookmarks "$(jq -nc --arg d "$html" '{data: $d}')"
assert_status 409 "bookmarks import while running"
assert_json '.code == "Conflict" and .details.reason == "profile_running"'
stop_and_wait "$inst"

BODY=$(brr profile bookmarks import jar -i "$SCRATCH/bookmarks.html" --json)
assert_json '.imported == 2' "two bookmarks imported"
brr profile bookmarks export jar -o "$SCRATCH/out.html" 2>/dev/null
CHECKS=$((CHECKS + 1))
grep -q 'https://example.com/a' "$SCRATCH/out.html" && grep -q 'https://example.org/b' "$SCRATCH/out.html" ||
    fail "exported bookmarks lack the imported URLs"

# 5. An ephemeral profile keeps nothing while stopped.
new_profile throwaway ephemeral
rest GET /v1/profiles/throwaway/cookies
assert_status 400 "cookies of a stopped ephemeral profile"
assert_json '.code == "InvalidArgument" and .details.field == "profile"'

finish "offline import, sent by the browser, running export, bookmarks import/export, ephemeral refused"
