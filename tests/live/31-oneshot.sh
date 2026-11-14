#!/usr/bin/env bash
# T3: one-shot captures through the ready pool, as raw HTTP, JSON ops, CLI and MCP.
# env: BRR_ONESHOT__POOL_SIZE=1
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
# The pool fills when the daemon starts, so it needs the Build that detect just registered.
stop_daemon
start_daemon
http_server "$M4_FIXTURES"
base=http://127.0.0.1:$HTTP_PORT

# The ready pool: an Instance of the `oneshot` profile is up before any request is made.
pool_running() {
    rest GET '/v1/instances?state=running'
    jq -e '[.instances[] | select(.profile_name == "oneshot")] | length >= 1' >/dev/null <<<"$BODY"
}
wait_until pool_running 60

raw() { # METHOD PATH [JSON]: raw response in $SCRATCH/raw.bin, headers in $HEADERS
    local out=$SCRATCH/raw.bin head=$SCRATCH/raw.head
    local args=(-sS --max-time 120 --unix-socket "$SOCK" -X "$1" -o "$out" -D "$head" -w '%{http_code}')
    [[ -n ${3-} ]] && args+=(-H 'content-type: application/json' --data "$3")
    STATUS=$(curl "${args[@]}" "http://localhost$2")
    HEADERS=$(<"$head")
    BODY=$(head -c 2000 "$out" 2>/dev/null || true)
}

raw GET "/v1/screenshot?url=$base/form.html&screen=800x600&theme=dark"
assert_status 200 "GET /v1/screenshot"
assert_eq "$(header_of content-type)" image/png "screenshot content type"
assert_eq "$(png_magic <"$SCRATCH/raw.bin")" "$PNG_MAGIC" "PNG signature"
assert_eq "$(od -An -tx1 -j16 -N8 "$SCRATCH/raw.bin" | tr -d ' \n')" 0000032000000258 "IHDR is 800x600"

raw POST /v1/markdown "{\"url\": \"$base/article.html\"}"
assert_status 200 "POST /v1/markdown"
assert_eq "$(header_of content-type | cut -d';' -f1)" text/markdown "markdown content type"
CHECKS=$((CHECKS + 1))
[[ $(head -c 2 "$SCRATCH/raw.bin") == "# " ]] || fail "markdown does not start with '# ': $BODY"

rest GET "/v1/links?url=$base/form.html"
assert_status 200 "GET /v1/links"
assert_json 'type == "array" and (map(.href) | any(endswith("/article.html")))' "links as a JSON array"

rest GET "/v1/screenshot?url=$base/form.html&return=url"
assert_status 200 "return=url"
assert_json '(.artifact_id | length > 0) and (.url | startswith("http"))'
size=$(jqv '.size')
url=$(jqv '.url')
curl -sS --max-time 30 "$url" -o "$SCRATCH/fetched.png"
assert_eq "$(stat -c %s "$SCRATCH/fetched.png")" "$size" "the URL serves the same bytes"
assert_eq "$(png_magic <"$SCRATCH/fetched.png")" "$PNG_MAGIC" "fetched PNG"

rest GET "/v1/screenshot?url=$base/form.html&bogus=1"
assert_status 400 "unknown query parameter"
assert_json '.code == "InvalidArgument"'
rest GET "/v1/pdf?url=$base/form.html&pdf_scale=9"
assert_status 400 "pdf_scale out of range"
assert_json '.details.field == "pdf_scale"'

brr shot screenshot "$base/form.html" -o "$SCRATCH/s.png" >/dev/null
CHECKS=$((CHECKS + 1))
[[ $(png_magic <"$SCRATCH/s.png") == "$PNG_MAGIC" ]] || fail "brr shot screenshot -o did not write a PNG"

mcp_tool oneshot_content "{\"url\": \"$base/form.html\"}"
assert_eq "$MCP_ERROR" false oneshot_content
assert_json '.inline_base64 | length > 0' "MCP inline_base64"
assert_eq "$(jqv '.inline_base64' | base64 -d | head -c 15)" '<!DOCTYPE html>' "inline content is the page HTML"

finish "one-shot screenshot, markdown, links, return=url, errors, CLI and MCP; the pool was warm"
