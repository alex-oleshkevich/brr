# Shared helpers for tests/live. Sourced by run.sh and by every NN-*.sh script.
# run.sh exports: BRR_BIN SCRATCH SOCK BIND XDG_* BRR_SERVER__BIND BRR_VAULT__KEY_SOURCE.

CHECKS=0
SLOW_PIDS=()
MCP_VERSION=2026-07-28
CHROME_NAMES=(google-chrome-stable google-chrome google-chrome-beta chromium chromium-browser brave-browser brave microsoft-edge-stable microsoft-edge)

# First log line is ~0.14s and listen ~0.24s when the host is free. After a script that
# filled RAM, paging in the debug binary can miss a 10s budget before that line exists.
DAEMON_START_TIMEOUT_S=30

fail() {
    echo "FAIL: $*" >&2
    [[ -n ${STATUS-} ]] && echo "  last REST status: $STATUS" >&2
    [[ -n ${BODY-} ]] && echo "  last body: ${BODY:0:1500}" >&2
    exit 1
}

# Exit 77 is how a script tells run.sh it was skipped.
skip() {
    echo "$*"
    exit 77
}

finish() {
    echo "ok: $1 ($CHECKS checks)"
}

ulid() {
    local tail
    tail=$(LC_ALL=C tr -dc '0-9A-HJKMNP-TV-Z' </dev/urandom | head -c 25 || true)
    echo "0$tail"
}

free_port() {
    node -e 'const s=require("net").createServer().listen(0,"127.0.0.1",()=>{console.log(s.address().port);s.close()})'
}

# wait_until CMD TIMEOUT_S: polls CMD (eval'd in this shell) every 0.2 s, fails the script on timeout.
wait_until() {
    local cmd=$1 timeout=$2 end=$((SECONDS + $2))
    until eval "$cmd"; do
        ((SECONDS < end)) || fail "timed out after ${timeout}s waiting for: $cmd"
        sleep 0.2
    done
}

# ---------------------------------------------------------------- assertions

assert_eq() {
    CHECKS=$((CHECKS + 1))
    [[ $1 == "$2" ]] || fail "${3:-assert_eq}: expected '$2', got '$1'"
}

assert_status() {
    CHECKS=$((CHECKS + 1))
    [[ ${STATUS-} == "$1" ]] || fail "${2:-status}: expected HTTP $1, got ${STATUS-none}"
}

# assert_json JQ_EXPR [MESSAGE]: asserts against $BODY; jq -e so null/false fail.
assert_json() {
    CHECKS=$((CHECKS + 1))
    jq -e "$1" >/dev/null <<<"$BODY" || fail "${2:-assert_json} ($1)"
}

# jqv JQ_EXPR: prints the raw value from $BODY; fails when it is null or missing.
jqv() {
    jq -er "$1" <<<"$BODY" || fail "jqv: nothing at $1"
}

# assert_log SUBSTRING: the daemon log holds the text a person would search for.
assert_log() {
    CHECKS=$((CHECKS + 1))
    grep -qF -- "$1" "$XDG_STATE_HOME/brr/daemon.log" || fail "daemon.log lacks: $1"
}

# ---------------------------------------------------------------- surfaces

# rest METHOD PATH [JSON] [CLIENT]: sets $STATUS, $BODY and $HEADERS (the response headers).
rest() {
    local method=$1 path=$2 data=${3-} client=${4-} out head
    out=$(mktemp -p "$SCRATCH")
    head=$(mktemp -p "$SCRATCH")
    local args=(-sS --max-time 120 --unix-socket "$SOCK" -X "$method" -o "$out" -D "$head" -w '%{http_code}')
    [[ -n $client ]] && args+=(-H "Brr-Client: $client")
    [[ -n $data ]] && args+=(-H 'content-type: application/json' --data "$data")
    STATUS=$(curl "${args[@]}" "http://localhost$path") || fail "transport failure: $method $path"
    BODY=$(<"$out")
    HEADERS=$(<"$head")
    rm -f "$out" "$head"
}

# header_of NAME: a response header of the last rest call (case-insensitive).
header_of() {
    sed -n "s/^$1: *//Ip" <<<"$HEADERS" | tr -d '\r' | head -1
}

brr() {
    "$BRR_BIN" "$@"
}

# mcp_http METHOD PARAMS_JSON: JSON-RPC POST to /mcp with the 2026-07-28 headers; sets MCP_STATUS and MCP_RAW.
mcp_http() {
    local method=$1 params=${2:-\{\} } name request
    params=$(jq -c --arg v "$MCP_VERSION" '. + {_meta: {
        "io.modelcontextprotocol/protocolVersion": $v,
        "io.modelcontextprotocol/clientInfo": {name: "live", version: "0"},
        "io.modelcontextprotocol/clientCapabilities": {}}}' <<<"$params")
    name=$(jq -r '.name // empty' <<<"$params")
    request=$(jq -nc --arg m "$method" --argjson p "$params" '{jsonrpc: "2.0", id: 1, method: $m, params: $p}')
    local args=(-sS --max-time 120 -X POST -H 'accept: application/json, text/event-stream'
        -H 'content-type: application/json' -H "MCP-Protocol-Version: $MCP_VERSION" -H "Mcp-Method: $method")
    [[ -n $name ]] && args+=(-H "Mcp-Name: $name")
    local out
    out=$(mktemp -p "$SCRATCH")
    MCP_STATUS=$(curl "${args[@]}" -o "$out" -w '%{http_code}' --data "$request" "http://$BIND/mcp") ||
        fail "transport failure: mcp $method"
    MCP_RAW=$(<"$out")
    rm -f "$out"
}

# mcp_tool TOOL ARGS_JSON: calls a tool; sets MCP_ERROR (true|false), MCP_TEXT and $BODY to the
# structuredContent, or for errors to the body JSON on the text's second line.
mcp_tool() {
    mcp_http tools/call "$(jq -nc --arg n "$1" --argjson a "${2:-\{\}}" '{name: $n, arguments: $a}')"
    [[ $MCP_STATUS == 200 ]] || fail "mcp $1: HTTP $MCP_STATUS: ${MCP_RAW:0:500}"
    jq -e '.result' >/dev/null <<<"$MCP_RAW" || fail "mcp $1: no result: ${MCP_RAW:0:500}"
    MCP_ERROR=$(jq -r '.result.isError // false' <<<"$MCP_RAW")
    MCP_TEXT=$(jq -r '.result.content[0].text // ""' <<<"$MCP_RAW")
    STATUS=
    if [[ $MCP_ERROR == true ]]; then
        BODY=$(sed -n '2,$p' <<<"$MCP_TEXT")
    else
        BODY=$(jq -c '.result.structuredContent' <<<"$MCP_RAW")
    fi
}

# set_settings JSON: PATCH /v1/settings, e.g. '{"idle.grace_s": 5}'.
set_settings() {
    rest PATCH /v1/settings "$1"
    assert_status 200 "set settings $1"
}

# events_for INSTANCE_ID: the Instance's events oldest first in $BODY.
events_for() {
    rest GET "/v1/events?instance_id=$1&order=asc&limit=1000"
    assert_status 200 "events for $1"
}

# state_trail INSTANCE_ID: "to:trigger" of each state change, one per line.
state_trail() {
    events_for "$1"
    jq -r '.events[] | select(.kind == "instance.state_changed") | "\(.data.to):\(.data.trigger)"' <<<"$BODY"
}

# ---------------------------------------------------------------- browsers

need_display() {
    [[ -n ${DISPLAY-} || -n ${WAYLAND_DISPLAY-} ]] || skip "neither DISPLAY nor WAYLAND_DISPLAY is set"
}

has_display() {
    [[ -n ${DISPLAY-} || -n ${WAYLAND_DISPLAY-} ]]
}

# cookie_server PORT: /set sets the brr cookie, /echo shows it in the title; killed on exit.
cookie_server() {
    node -e '
const http = require("http");
http.createServer((req, res) => {
  const cookie = /(?:^|; )brr=([^;]*)/.exec(req.headers.cookie || "");
  res.setHeader("content-type", "text/html");
  if (req.url === "/set") {
    res.setHeader("set-cookie", "brr=live; Max-Age=3600; Path=/");
    res.end("<title>set</title>");
  } else {
    res.end("<title>cookie:" + (cookie ? cookie[1] : "none") + "</title>");
  }
}).listen(+process.argv[1], "127.0.0.1");' "$1" &
    SLOW_PIDS+=($!)
    wait_until "(echo > /dev/tcp/127.0.0.1/$1) 2>/dev/null" 10
}

have_chrome() {
    local name
    for name in "${CHROME_NAMES[@]}"; do
        command -v "$name" >/dev/null && return 0
    done
    return 1
}

need_chrome() {
    have_chrome || skip "no Chromium-family browser on PATH and BRR_TEST_CHROME unset"
}

detect_builds() {
    rest POST /v1/builds/detect '{}'
    assert_status 200 build.detect
    assert_json '.builds | length >= 1' "detect registered a Build"
}

# new_profile NAME KIND [TEMPLATE]: creates a profile, prints nothing; $BODY is the profile.
new_profile() {
    rest POST /v1/profiles "$(jq -nc --arg n "$1" --arg k "$2" --arg t "${3:-chromium-headless}" '{name: $n, kind: $k, template: $t}')"
    assert_status 200 "create profile $1"
}

# launch PROFILE [CLIENT] [URL]: launches; $BODY is {instance, tab, lease, ...}.
launch() {
    rest POST "/v1/profiles/$1/launch" "$(jq -nc --arg u "${3:-data:text/html,<title>a</title>}" '{url: $u}')" "${2-}"
    assert_status 200 "launch $1"
}

# open_tab INSTANCE_ID: opens a Tab and sets $TAB_ID once it is ready.
open_tab() {
    rest POST "/v1/instances/$1/tabs" "$(jq -nc '{url: "data:text/html,<title>t</title>"}')"
    assert_status 200 tab.open
    TAB_ID=$(jqv '.tab.id')
    tab_ready() {
        rest GET "/v1/tabs/$TAB_ID"
        [[ $(jq -r .state <<<"$BODY") == ready ]]
    }
    wait_until tab_ready 10
}

stop_instance() {
    # An operator stop: `force` overrides other Clients' Leases (LC9).
    rest POST "/v1/instances/$1/stop" '{"force": true}'
    assert_status 200 "stop $1"
}

instance_state() {
    rest GET "/v1/instances/$1"
    jq -r '.state' <<<"$BODY"
}

# marker_pids ID...: pids of this uid whose environment carries BRR_INSTANCE_ID=<id>.
marker_pids() {
    local patterns env_file pid
    patterns=$(mktemp -p "$SCRATCH")
    printf 'BRR_INSTANCE_ID=%s\n' "$@" >"$patterns"
    for env_file in /proc/[0-9]*/environ; do
        [[ -O $env_file ]] || continue
        if cat "$env_file" 2>/dev/null | tr '\0' '\n' | grep -qxFf "$patterns"; then
            pid=${env_file#/proc/}
            echo "${pid%/environ}"
        fi
    done
    rm -f "$patterns"
}

no_marker_pids() {
    [[ -z $(marker_pids "$@") ]]
}

# slow_server PORT: an HTTP server that accepts and never answers; killed on exit.
slow_server() {
    node -e 'require("http").createServer(() => {}).listen(+process.argv[1], "127.0.0.1")' "$1" &
    SLOW_PIDS+=($!)
    wait_until "(echo > /dev/tcp/127.0.0.1/$1) 2>/dev/null" 10
}

cleanup_script() {
    local pid
    stop_test_chrome
    stop_second_daemon
    for pid in "${SLOW_PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    jobs -p | xargs -r kill 2>/dev/null || true
}
trap cleanup_script EXIT

# ---------------------------------------------------------------- daemon

daemon_pid() {
    cat "$SCRATCH/daemon.pid" 2>/dev/null
}

daemon_alive() {
    local pid
    pid=$(daemon_pid) && [[ -n $pid ]] && kill -0 "$pid" 2>/dev/null
}

daemon_answers() {
    curl -sf --max-time 2 --unix-socket "$SOCK" http://localhost/v1/system/info >/dev/null 2>&1
}

# start_daemon: runs `brr daemon run` in the background, stderr appended to daemon.log.
# DAEMON_PATH, when set, is the daemon's PATH instead of this shell's.
start_daemon() {
    mkdir -p "$XDG_STATE_HOME/brr"
    (PATH=${DAEMON_PATH:-$PATH}; exec "$BRR_BIN" daemon run 2>>"$XDG_STATE_HOME/brr/daemon.log" >/dev/null) &
    echo $! >"$SCRATCH/daemon.pid"
    disown
    wait_until 'daemon_answers' "$DAEMON_START_TIMEOUT_S"
}

# stop_daemon: SIGTERM, bounded wait, SIGKILL as the last resort. Returns 1 when it had to kill.
stop_daemon() {
    local pid
    daemon_alive || return 0
    pid=$(daemon_pid)
    kill -TERM "$pid"
    local end=$((SECONDS + ${1:-30}))
    while kill -0 "$pid" 2>/dev/null; do
        if ((SECONDS >= end)); then
            kill -KILL "$pid" 2>/dev/null || true
            return 1
        fi
        sleep 0.2
    done
}

# ---------------------------------------------------------------- M3: proxies, extensions, files

LIVE_SUPPORT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/support

# support_server SCRIPT PORT [ARG...]: runs a node support script in the background, killed on exit.
support_server() {
    local script=$1 port=$2
    shift
    node "$LIVE_SUPPORT/$script" "$@" &
    SLOW_PIDS+=($!)
    wait_until "(echo > /dev/tcp/127.0.0.1/$port) 2>/dev/null" 10
}

# upstream_proxy PORT USER PASS: Basic-auth HTTP proxy that dials from 127.0.0.2.
upstream_proxy() {
    support_server upstream-proxy.js "$@"
}

# peer_server PORT: <title>peer:<remote address></title> at /, cookie routes /set and /echo.
peer_server() {
    support_server peer-server.js "$1"
}

# trace_server PORT: answers ip=203.0.113.7 loc=NL, for proxy.check_url.
trace_server() {
    support_server trace-server.js "$1"
}

# can_bind_loopback2: false in a container without the full 127.0.0.0/8 loopback.
can_bind_loopback2() {
    node -e 'const s=require("net").createServer().listen(0,"127.0.0.2",()=>process.exit(0));s.on("error",()=>process.exit(1))'
}

# make_ext DIR: a minimal MV3 extension whose content script titles http://127.0.0.1/* pages "ext-ok".
make_ext() {
    mkdir -p "$1"
    cat >"$1/manifest.json" <<'JSON'
{
  "manifest_version": 3,
  "name": "brr live ext",
  "version": "1.0.0",
  "content_scripts": [
    {"matches": ["http://127.0.0.1/*"], "js": ["content.js"], "run_at": "document_idle"}
  ]
}
JSON
    echo 'document.title = "ext-ok";' >"$1/content.js"
}

# wait_title TAB TITLE SECONDS: polls tab.get until the title matches.
wait_title() {
    wait_until "rest GET /v1/tabs/$1; [[ \$(jq -r .title <<<\"\$BODY\") == '$2' ]]" "$3"
}

# profile_set PROFILE JQ_FILTER [JQ_ARGS...]: updates a Profile's settings by applying the filter to
# the current ones (profile.update replaces the whole settings object); $BODY is the PATCH response.
profile_set() {
    local profile=$1 filter=$2
    shift 2
    rest GET "/v1/profiles/$profile"
    assert_status 200 "read $profile"
    rest PATCH "/v1/profiles/$profile" "$(jq -c "$@" "{settings: (.settings | $filter)}" <<<"$BODY")"
    assert_status 200 "update $profile settings"
}

# ---------------------------------------------------------------- M4: fixtures, rustfs, /cdp

M4_FIXTURES=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/fixtures/m4
RUSTFS_URL=${BRR_LIVE_RUSTFS_URL:-http://127.0.0.1:9900}

# need_rustfs: skip unless the live rustfs answers (any HTTP status).
need_rustfs() {
    local code
    code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 3 "$RUSTFS_URL/" || true)
    [[ $code =~ ^[1-5][0-9][0-9]$ ]] || skip "rustfs is not answering at $RUSTFS_URL"
}

# http_server DIR [--download]: serves DIR on a free loopback port; the port is left in $HTTP_PORT.
# With --download every *.bin is sent as an attachment.
http_server() {
    local dir=$1 mode=${2-}
    HTTP_PORT=$(free_port)
    python3 "$LIVE_SUPPORT/http-server.py" "$HTTP_PORT" "$dir" $mode >/dev/null 2>&1 &
    SLOW_PIDS+=($!)
    wait_until "(echo > /dev/tcp/127.0.0.1/$HTTP_PORT) 2>/dev/null" 10
}

# cdp_ws URL SCRIPT_JSON: runs cdp-client.ts, prints every received frame as a JSON line.
cdp_ws() {
    bun "$(dirname "${BASH_SOURCE[0]}")/cdp-client.ts" "$1" "$2"
}

# tab_call TAB TOOL JSON [CLIENT]: POST /v1/tabs/TAB/TOOL; sets $STATUS and $BODY.
tab_call() {
    local data=${3-}
    [[ -n $data ]] || data='{}'
    rest POST "/v1/tabs/$1/$2" "$data" "${4-}"
}

# png_magic: the first 8 bytes of stdin as hex; 89504e470d0a1a0a for a PNG.
png_magic() {
    head -c 8 | od -An -tx1 | tr -d ' \n'
}
PNG_MAGIC=89504e470d0a1a0a

# ---------------------------------------------------------------- M5: Camoufox, Xvfb, live view

M5_REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)

have_camoufox() {
    [[ -n ${BRR_TEST_CAMOUFOX-} && -x ${BRR_TEST_CAMOUFOX-} && -n ${BRR_TEST_FPGEN_MODEL-} && -d ${BRR_TEST_FPGEN_MODEL-} ]]
}

# need_camoufox: skip unless the test Camoufox and the fpgen model are there.
need_camoufox() {
    [[ -n ${BRR_TEST_CAMOUFOX-} && -x ${BRR_TEST_CAMOUFOX-} ]] || skip "BRR_TEST_CAMOUFOX is not an executable Camoufox"
    [[ -n ${BRR_TEST_FPGEN_MODEL-} && -d ${BRR_TEST_FPGEN_MODEL-} ]] || skip "BRR_TEST_FPGEN_MODEL is not a directory"
}

need_xvfb() {
    command -v Xvfb >/dev/null || skip "Xvfb is not on PATH"
}

# seed_camoufox: with the daemon stopped, registers the test Camoufox as an installed Build and its
# fonts and fpgen model as assets in the scratch brr.db, then restarts the daemon. Nothing is
# written outside the scratch dirs, and the compatibility gate does not run for a seeded Build.
seed_camoufox() {
    local install version pin
    install=$(dirname "$BRR_TEST_CAMOUFOX")
    version=$(sed -n 's/^Version=//p' "$install/application.ini" | head -1)
    pin=$(jq -r .sha256 "$M5_REPO/crates/brr-camoufox/data/fpgen-model.json")
    [[ -n $version && -n $pin && -d $install/fonts ]] || fail "seed_camoufox: no Version, pin or fonts dir"
    stop_daemon 30 || fail "seed_camoufox: the daemon ignored SIGTERM"
    python3 - "$XDG_DATA_HOME/brr/brr.db" "$BRR_TEST_CAMOUFOX" "$version" "$install/fonts" "$BRR_TEST_FPGEN_MODEL" "$pin" \
        "$(ulid)" "$(ulid)" "$(ulid)" <<'PY'
import json, sqlite3, sys, time
db, binary, version, fonts, model, pin, build_id, fonts_id, model_id = sys.argv[1:]
now = int(time.time() * 1000)
caps = ["screencast", "unpacked_extensions", "har", "request_interception", "fingerprint_config"]
con = sqlite3.connect(db)
con.execute(
    "INSERT INTO build (id, source, engine, name, version, channel, path, state, sha256, size_bytes, capabilities, installed_at, last_referenced_at)"
    " VALUES (?, 'camoufox', 'camoufox', 'Camoufox', ?, 'latest-compatible', ?, 'installed', NULL, 0, ?, ?, NULL)",
    (build_id, version, binary, json.dumps(caps), now))
con.executemany(
    "INSERT INTO asset (id, kind, version, path, updated_at) VALUES (?, ?, ?, ?, ?)",
    [(fonts_id, "fonts_bundle", "test", fonts, now), (model_id, "fpgen_model", pin, model, now)])
con.commit()
con.close()
PY
    start_daemon
}

# ws_client URL OUT [SECONDS]: runs ws-client.ts in the background with a fifo for input; sets WS_PID.
# Messages for the socket go through ws_send; every received message lands in OUT, one JSON line each.
ws_client() {
    WS_IN=$(mktemp -u -p "$SCRATCH")
    mkfifo "$WS_IN"
    bun "$(dirname "${BASH_SOURCE[0]}")/ws-client.ts" "$1" "${3:-60}" <"$WS_IN" >"$2" 2>&1 &
    WS_PID=$!
    SLOW_PIDS+=($WS_PID)
    exec {WS_FD}>"$WS_IN"
}

# ws_send JSON: one message to the socket opened by ws_client.
ws_send() {
    echo "$1" >&"$WS_FD" 2>/dev/null || true
}

# ws_close: ends the input, which closes the socket.
ws_close() {
    exec {WS_FD}>&-
    wait "$WS_PID" 2>/dev/null || true
}

# ws_probe PATH [CLIENT_QUERY]: a WebSocket upgrade request over TCP that is not completed; sets $STATUS
# (101 when the server accepts) and $BODY (the error body of a refusal).
ws_probe() {
    local out
    out=$(mktemp -p "$SCRATCH")
    STATUS=$(curl -s --max-time 3 -o "$out" -w '%{http_code}' \
        -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' \
        -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' "http://$BIND$1" || true)
    BODY=$(<"$out")
    rm -f "$out"
}

# ---------------------------------------------------------------- M6: fixtures

M6_FIXTURES=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/fixtures

# start_fixture NAME PORT: runs fixtures/NAME.mjs on PORT in the background, killed on exit.
start_fixture() {
    node "$M6_FIXTURES/$1.mjs" "$2" &
    SLOW_PIDS+=($!)
    wait_until "(echo > /dev/tcp/127.0.0.1/$2) 2>/dev/null" 10
}

# ---------------------------------------------------------------- M7: live browsers, backups

# start_test_chrome [ARG...]: the test Chrome with a scratch profile and a debugging port of its own;
# never the user's browser. Sets TEST_CHROME_PID, TEST_CHROME_PORT and TEST_CHROME_WS.
start_test_chrome() {
    [[ -n ${BRR_TEST_CHROME-} && -x ${BRR_TEST_CHROME-} ]] || skip "BRR_TEST_CHROME is not an executable Chrome"
    local profile=$SCRATCH/chrome
    mkdir -p "$profile"
    rm -f "$profile/DevToolsActivePort"
    "$BRR_TEST_CHROME" --remote-debugging-port=0 --user-data-dir="$profile" --no-first-run \
        --no-default-browser-check --headless=new --no-sandbox "$@" about:blank >"$SCRATCH/test-chrome.log" 2>&1 &
    TEST_CHROME_PID=$!
    wait_until "[[ -s $profile/DevToolsActivePort ]]" 20
    TEST_CHROME_PORT=$(sed -n 1p "$profile/DevToolsActivePort")
    TEST_CHROME_WS=ws://127.0.0.1:$TEST_CHROME_PORT$(sed -n 2p "$profile/DevToolsActivePort")
    export TEST_CHROME_PID TEST_CHROME_PORT TEST_CHROME_WS
}

stop_test_chrome() {
    [[ -n ${TEST_CHROME_PID-} ]] || return 0
    kill "$TEST_CHROME_PID" 2>/dev/null || return 0
    local end=$((SECONDS + 10))
    while kill -0 "$TEST_CHROME_PID" 2>/dev/null && ((SECONDS < end)); do sleep 0.2; done
    kill -KILL "$TEST_CHROME_PID" 2>/dev/null || true
}

# extension_id DIR: the id Chrome gives an unpacked extension loaded from DIR (first 32 hex digits of
# sha256 of the absolute path, 0-f mapped to a-p).
extension_id() {
    local path
    path=$(realpath "$1")
    printf %s "$path" | sha256sum | head -c 32 | tr '0-9a-f' 'a-p'
}

# need_s3: skip unless the live rustfs answers.
need_s3() {
    need_rustfs
}

# start_second_daemon: a second daemon on a fresh data root and port, with the same browser PATH.
# Sets D2_BIND and D2_SOCK; use rest2 and brr2 against it. Stopped by the EXIT trap.
start_second_daemon() {
    D2_DIR=$SCRATCH/second
    mkdir -p "$D2_DIR"/{config,data,state,runtime}
    D2_BIND=127.0.0.1:$(free_port)
    D2_SOCK=$D2_DIR/runtime/brr/brr.sock
    mkdir -p "$D2_DIR/state/brr"
    (
        export XDG_CONFIG_HOME=$D2_DIR/config XDG_DATA_HOME=$D2_DIR/data XDG_STATE_HOME=$D2_DIR/state \
            XDG_RUNTIME_DIR=$D2_DIR/runtime BRR_SERVER__BIND=$D2_BIND
        exec "$BRR_BIN" daemon run 2>>"$D2_DIR/state/brr/daemon.log" >/dev/null
    ) &
    D2_PID=$!
    wait_until "curl -sf --max-time 2 --unix-socket $D2_SOCK http://localhost/v1/system/info >/dev/null 2>&1" 15
}

stop_second_daemon() {
    [[ -n ${D2_PID-} ]] || return 0
    kill -TERM "$D2_PID" 2>/dev/null || return 0
    local end=$((SECONDS + 30))
    while kill -0 "$D2_PID" 2>/dev/null && ((SECONDS < end)); do sleep 0.2; done
    kill -KILL "$D2_PID" 2>/dev/null || true
}

# rest2 METHOD PATH [JSON]: like rest, against the second daemon.
rest2() {
    local method=$1 path=$2 data=${3-} out
    out=$(mktemp -p "$SCRATCH")
    local args=(-sS --max-time 120 --unix-socket "$D2_SOCK" -X "$method" -o "$out" -w '%{http_code}')
    [[ -n $data ]] && args+=(-H 'content-type: application/json' --data "$data")
    STATUS=$(curl "${args[@]}" "http://localhost$path") || fail "transport failure (second daemon): $method $path"
    BODY=$(<"$out")
    rm -f "$out"
}

# brr2 ARG...: the CLI against the second daemon.
brr2() {
    XDG_CONFIG_HOME=$D2_DIR/config XDG_DATA_HOME=$D2_DIR/data XDG_STATE_HOME=$D2_DIR/state \
        XDG_RUNTIME_DIR=$D2_DIR/runtime BRR_SERVER__BIND=$D2_BIND "$BRR_BIN" "$@"
}

# A fixture page for attach tests: <title>NAME</title> on a loopback http server ($HTTP_PORT).
M7_FIXTURES=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/fixtures/m7

# Live view helpers. They read $tab (the Tab), $holder (the Client that may evaluate in it) and $out
# (the socket log of ws_client).
# frames_in OUT: the number of frame messages in a socket log.
frames_in() {
    grep -c '"type":"frame"' "$1" || true
}

# last_frame OUT: the newest frame line.
last_frame() {
    grep '"type":"frame"' "$1" | tail -1
}

# center SELECTOR: the element's center in frame pixels, "x y", from the newest frame's scale.
center() {
    local frame rect
    tab_call "$tab" evaluate "$(jq -nc --arg s "$1" '{expression: "(() => { const r = document.querySelector(\($s | @json)).getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()"}')" "$holder"
    assert_status 200 "locate $1"
    rect=$BODY
    frame=$(last_frame "$out")
    jq -nr --argjson r "$rect" --argjson f "$frame" \
        '"\($r.result[0] * $f.width / $f.deviceWidth) \($r.result[1] * $f.height / $f.deviceHeight)"'
}

# click_at SELECTOR: a left press and release over the element, through the socket.
click_at() {
    local x y
    read -r x y <<<"$(center "$1")"
    ws_send "{\"type\":\"mouse\",\"event\":\"down\",\"x\":$x,\"y\":$y,\"button\":\"left\",\"click_count\":1}"
    ws_send "{\"type\":\"mouse\",\"event\":\"up\",\"x\":$x,\"y\":$y,\"button\":\"left\",\"click_count\":1}"
}

# ws_has OUT JQ_CONDITION: some message in a socket log satisfies the condition, e.g. '.type == "dialog"'.
ws_has() {
    jq -es "any(.[]; $2)" "$1" >/dev/null 2>&1
}
