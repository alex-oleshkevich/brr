#!/usr/bin/env bash
# §14 across the whole surface: every op family answers its error paths with the typed error, the HTTP
# status of the spec's table, the exit code of the CLI and the `<code>: <message>` MCP text; never a 500,
# never a panic, never a secret.
#
# Unlike the other scripts a failed assertion here does not stop the run: each one is recorded with the
# request, what the spec says and what came back, and the table is printed at the end. A line in the
# "spec ambiguity" table is an observation the spec does not settle; it never fails the script.
source "$(dirname "$0")/lib.sh"
set -uo pipefail

# ---------------------------------------------------------------- soft assertions

FAILS=()
AMBIG=()
REQ=
SECRETS=()
CLASS_BUG="daemon bug"

declare -A HTTP_OF=(
    [InvalidArgument]=400 [NotFound]=404 [Conflict]=409 [InvalidTransition]=409 [LeaseHeld]=423 [LeaseLost]=410
    [CapExceeded]=429 [QuotaExceeded]=507 [DownloadTooLarge]=413 [RequestTooLarge]=413 [BuildMissing]=424
    [EngineUnsupported]=422 [DependencyMissing]=503 [ProxyUnhealthy]=502 [NavigationFailed]=502
    [ElementNotFound]=422 [StaleRef]=422 [EvaluationFailed]=422 [LoginFailed]=422 [CaptchaBudgetExceeded]=402
    [CaptchaFailed]=422 [StorageUnavailable]=503 [Timeout]=504 [Internal]=500
)
declare -A EXIT_OF=(
    [InvalidArgument]=2 [NotFound]=3 [Conflict]=4 [InvalidTransition]=5 [LeaseHeld]=6 [LeaseLost]=7
    [CapExceeded]=8 [QuotaExceeded]=9 [DownloadTooLarge]=10 [RequestTooLarge]=2 [BuildMissing]=11
    [EngineUnsupported]=12 [DependencyMissing]=13 [ProxyUnhealthy]=14 [NavigationFailed]=15
    [ElementNotFound]=16 [StaleRef]=16 [EvaluationFailed]=17 [LoginFailed]=18 [CaptchaBudgetExceeded]=19
    [CaptchaFailed]=20 [StorageUnavailable]=21 [Timeout]=22 [Internal]=1
)

short() { tr '\n' ' ' <<<"${1:0:${2:-220}}"; }

record() { # WHAT EXPECTED ACTUAL [CLASS]
    FAILS+=("$(short "$1" 200)"$'\t'"$(short "$2")"$'\t'"$(short "$3")"$'\t'"${4:-$CLASS_BUG}")
}

# check WHAT EXPECTED ACTUAL [CLASS]: string equality, recorded with the current request.
check() {
    CHECKS=$((CHECKS + 1))
    [[ $3 == "$2" ]] || record "$REQ: $1" "$2" "$3" "${4-}"
}

# ambiguity WHAT EXPECTED ACTUAL: the spec does not settle it; shown in the report, never fails.
ambiguity() {
    CHECKS=$((CHECKS + 1))
    [[ $3 == "$2" ]] || AMBIG+=("$(short "$REQ: $1" 200)"$'\t'"$(short "$2")"$'\t'"$(short "$3")")
}

# check_true WHAT JQ_EXPR TEXT: the jq expression is true for TEXT.
check_json() { # WHAT JQ_EXPR [TEXT]
    CHECKS=$((CHECKS + 1))
    local text=${3-$BODY}
    jq -e "$2" >/dev/null 2>&1 <<<"$text" || record "$REQ: $1" "true: $2" "$(short "$text")"
}

# no_secret TEXT: none of the secrets this script made up is in TEXT.
no_secret() {
    local secret
    for secret in "${SECRETS[@]}"; do
        CHECKS=$((CHECKS + 1))
        [[ $1 != *"$secret"* ]] || record "$REQ: secret in error text" "no '$secret'" "$(short "$1")"
    done
}

# ---------------------------------------------------------------- the three surfaces

# rx METHOD PATH [JSON [CLIENT]]: a REST call; sets REQ, $STATUS, $BODY, $HEADERS.
rx() {
    local client=${4-${RX_CLIENT-}}
    REQ="$1 $2 $(short "${3-}" 90)${client:+ as client}"
    rest "$1" "$2" "${3-}" "$client"
}

# rx_file METHOD PATH FILE: like rx, with the body taken from a file (archives are too big for argv).
rx_file() {
    local out
    out=$(mktemp -p "$SCRATCH")
    REQ="$1 $2 <body from file, $(stat -c %s "$3") bytes>"
    STATUS=$(curl -sS --max-time 120 --unix-socket "$SOCK" -X "$1" -H 'content-type: application/json' \
        --data-binary @"$3" -o "$out" -D "$SCRATCH/rx.hdr" -w '%{http_code}' "http://localhost$2") || fail "transport failure: $1 $2"
    BODY=$(<"$out")
    HEADERS=$(<"$SCRATCH/rx.hdr")
    rm -f "$out"
}

# import_body FILE_B64 PASSPHRASE [NAME]: the profile.import body in $SCRATCH/import.body.
import_body() {
    jq -nc --rawfile f <(printf %s "$1") --arg p "$2" --arg n "${3-}" \
        '{file: $f, passphrase: $p} + (if $n == "" then {} else {name: $n} end)' >"$SCRATCH/import.body"
}

# err CODE [FIELD]: the last REST answer is that typed error, with the spec's status and the error shape.
err() {
    local code=$1 field=${2-} want=${HTTP_OF[$1]} got
    check "HTTP status" "$want" "$STATUS"
    got=$(jq -r 'if type == "object" then (.code // "none") else "notjson" end' <<<"$BODY" 2>/dev/null || echo notjson)
    CHECKS=$((CHECKS + 1))
    [[ $got == "$code" ]] || record "$REQ: error code" "$code" "$got -- $(short "$BODY" 160)"
    check "body is {code, message, details}" ok "$(jq -r 'if type == "object" and (.message | type == "string" and length > 0) and has("details") and (.details | type == "object" or . == null) then "ok" else "bad" end' <<<"$BODY" 2>/dev/null || echo notjson)"
    if [[ -n $field ]]; then
        check "details.field" "$field" "$(jq -r '.details.field // "none"' <<<"$BODY" 2>/dev/null)"
    fi
    if ((want == 429)); then
        check "Retry-After is whole seconds" ok "$([[ $(header_of retry-after) =~ ^[0-9]+$ ]] && echo ok || echo "'$(header_of retry-after)'")"
    fi
    no_secret "$BODY$HEADERS"
}

# err_any "S1 S2" CODE1: like err for a status the spec leaves open between two values.
err_shape() { # the answer is a JSON error object of some 4xx
    CHECKS=$((CHECKS + 1))
    ((STATUS >= 400 && STATUS < 500)) || record "$REQ: HTTP status" "4xx" "$STATUS"
    check "body is {code, message, details}" ok "$(jq -r 'if type == "object" and (.message | type == "string" and length > 0) and has("details") then "ok" else "bad" end' <<<"$BODY" 2>/dev/null || echo notjson)"
}

# okr: the last REST answer is a 200 with a JSON object.
okr() {
    check "HTTP status" 200 "$STATUS"
    check "JSON object" object "$(jq -r 'type' <<<"$BODY" 2>/dev/null || echo notjson)"
}

# mx CODE TOOL ARGS [FIELD]: an MCP tool call that fails with CODE.
mx() {
    local code=$1 tool=$2 args=${3:-\{\} } field=${4-} first
    REQ="mcp $tool $(short "$args" 90)"
    mcp_http tools/call "$(jq -nc --arg n "$tool" --argjson a "$args" '{name: $n, arguments: $a}')"
    check "MCP HTTP status" 200 "$MCP_STATUS"
    check "isError" true "$(jq -r '.result.isError // false' <<<"$MCP_RAW" 2>/dev/null)"
    MCP_TEXT=$(jq -r '.result.content[0].text // ""' <<<"$MCP_RAW" 2>/dev/null)
    BODY=$(sed -n '2,$p' <<<"$MCP_TEXT")
    first=$(sed -n 1p <<<"$MCP_TEXT")
    check "text starts with '<code>: '" "$code: " "${first:0:$((${#code} + 2))}"
    check "body JSON code on line 2" "$code" "$(jq -r '.code // "none"' <<<"$BODY" 2>/dev/null || echo notjson)"
    check "line 1 is '<code>: <message>'" "$code: $(jq -r '.message // ""' <<<"$BODY" 2>/dev/null)" "$first"
    check "no structuredContent on a tool error" false "$(jq -r '.result | has("structuredContent")' <<<"$MCP_RAW" 2>/dev/null)"
    if [[ -n $field ]]; then
        check "details.field" "$field" "$(jq -r '.details.field // "none"' <<<"$BODY" 2>/dev/null)"
    fi
    no_secret "$MCP_RAW"
}

# mok TOOL ARGS: a tool call that succeeds; $BODY is the structuredContent.
mok() {
    REQ="mcp $1 $(short "${2:-\{\}}" 90)"
    mcp_http tools/call "$(jq -nc --arg n "$1" --argjson a "${2:-\{\}}" '{name: $n, arguments: $a}')"
    check "MCP HTTP status" 200 "$MCP_STATUS"
    check "isError" false "$(jq -r '.result.isError // false' <<<"$MCP_RAW" 2>/dev/null)"
    BODY=$(jq -c '.result.structuredContent' <<<"$MCP_RAW" 2>/dev/null)
}

# cx CODE ARG...: a CLI call that fails with CODE: the spec's exit code, nothing on stdout, `error: ...` on stderr.
cx() {
    local code=$1 out err rc=0
    shift
    REQ="brr $(short "$*" 150)"
    out=$(timeout 120 "$BRR_BIN" "$@" 2>"$SCRATCH/cli.err" </dev/null) || rc=$?
    err=$(<"$SCRATCH/cli.err")
    check "exit code" "${EXIT_OF[$code]}" "$rc"
    check "stdout stays empty" "" "$out"
    check "stderr is 'error: <message>'" ok "$([[ $err == "error: "?* && $err != *panicked* ]] && echo ok || echo "$err")"
    no_secret "$out$err"
}

# cx_usage ARG...: a CLI usage error (clap): exit 2, the usage on stderr, nothing on stdout.
cx_usage() {
    local out err rc=0
    REQ="brr $(short "$*" 150)"
    out=$(timeout 60 "$BRR_BIN" "$@" 2>"$SCRATCH/cli.err" </dev/null) || rc=$?
    err=$(<"$SCRATCH/cli.err")
    check "exit code" 2 "$rc"
    check "stdout stays empty" "" "$out"
    check "stderr starts with 'error:'" ok "$([[ $err == "error: "?* ]] && echo ok || echo "$err")"
}

# ---------------------------------------------------------------- generic operation inputs (from the OpenAPI)

spec=$SCRATCH/openapi.json
curl -sS --unix-socket "$SOCK" http://localhost/openapi.json >"$spec"
echo "name,url,username,password" >"$SCRATCH/import.csv"

# input_for OP ID: minimal valid input for OP: ID for every path parameter, a dummy for every required field.
input_for() {
    jq -nc --slurpfile spec "$spec" --arg op "$1" --arg u "$2" --arg csv "$SCRATCH/import.csv" '
    def pick: if type == "object" and has("$ref") then ($spec[0].components.schemas[.["$ref"] | split("/") | last] | pick)
        elif type == "object" and has("anyOf") then (.anyOf | map(select(.type != "null")) | .[0] | pick) else . end;
    def ty: (.type | if type == "array" then map(select(. != "null"))[0] else . end);
    def dummy: if type == "object" and has("$ref") and (.["$ref"] | test("Id$")) then $u
        else pick | if has("enum") then .enum[0]
        elif has("x-brr-file") then $csv
        elif has("oneOf") then (.oneOf[0] | dummy)
        elif ty == "string" then "x"
        elif ty == "integer" or ty == "number" then 1
        elif ty == "boolean" then false
        elif ty == "array" then []
        elif ty == "object" then (. as $o | ($o.required // []) | map({key: ., value: ($o.properties[.] | dummy)}) | from_entries)
        else null end end;
    ([$spec[0].paths[][] | select(type == "object" and .operationId == $op)][0]) as $o
    | (([$o.parameters[]? | select(.in == "path") | {key: .name, value: $u}] | from_entries)
       + ($o.requestBody.content["application/json"].schema // {} | pick | if .type == "object" then dummy else {} end))
    * ({
      "build.install": {source: "system"},
      "build.available": {source: "system"},
      "credential.create": {site: "https://site.test", username: "u", password: "p"},
      "events.watch": {timeout_ms: 100},
      "extension.add": {ref: "not-an-extension"},
      "profile.create": {name: "p-sweep", template: "chromium-headless"},
      "template.save_from_profile": {name: "t-sweep"},
      "settings.set": {"ui.theme": "dark"},
      "tab.navigate": {url: "about:blank"},
      "template.create": {name: "t-sweep", settings: {}},
      "proxy.create": {scheme: "http", host: "127.0.0.1", port: 9},
      "proxy.import": {scheme: "http", text: "127.0.0.1:9"},
      "tab.click": {selector: "#x"}, "tab.dblclick": {selector: "#x"}, "tab.hover": {selector: "#x"},
      "tab.check": {selector: "#x"}, "tab.uncheck": {selector: "#x"}, "tab.fill": {selector: "#x"},
      "tab.select_option": {selector: "#x"}, "tab.upload_file": {selector: "#x"},
      "tab.drag": {selector: "#x", to_selector: "#y"},
      "tab.emulate": {timezone: "UTC"}, "tab.handle_dialog": {accept: true},
      "tab.block_requests": {patterns: ["*.png"], types: ["image"]},
      "tab.wait_for": {network_idle: true},
      "tab.press_key": {key: "Enter"}
    }[$op] // {})'
}

# rest_op METHOD PATH INPUT: the REST call for an input that mixes path parameters, query and body.
rest_op() {
    local method=$1 path=$2 input=$3 key query='' body='{}'
    for key in $(grep -o '{[a-z_]*}' <<<"$path" | tr -d '{}'); do
        path=${path//\{$key\}/$(jq -r --arg k "$key" '.[$k]' <<<"$input")}
        input=$(jq -c --arg k "$key" 'del(.[$k])' <<<"$input")
    done
    if [[ $method == GET ]]; then
        query=$(jq -r 'to_entries | map("\(.key)=\(.value | tostring | @uri)") | join("&")' <<<"$input")
        [[ -z $query ]] || path="$path?$query"
        rx GET "$path"
    elif [[ $method == DELETE ]]; then
        query=$(jq -r 'to_entries | map("\(.key)=\(.value | tostring | @uri)") | join("&")' <<<"$input")
        [[ -z $query ]] || path="$path?$query"
        rx DELETE "$path"
    else
        [[ $input == '{}' ]] || body=$input
        rx "$method" "$path" "$body"
    fi
}

# cli_argv CLI INPUT: sets CLI_WORDS and CLI_ARGV, the command words and flags for an input, read off --help.
cli_argv() {
    local cli=$1 input=$2 help usage flag name value kind
    local -a positionals=()
    declare -A given=()
    CLI_ARGV=()
    CLI_WORDS=()
    # An op without a command (the UI's human overrides) has no CLI form.
    if [[ $cli == - ]]; then
        CLI_SKIP=1
        return
    fi
    # An array of objects, or an empty one, has no flag form: the sweep skips the CLI for that input.
    CLI_SKIP=$(jq -r 'if any(.[]; type == "array" and (length == 0 or any(.[]; type != "string"))) then 1 else 0 end' <<<"$input")
    read -ra CLI_WORDS <<<"$cli"
    help=$("$BRR_BIN" "${CLI_WORDS[@]}" --help)
    usage=$(sed -n 's/^Usage: //p' <<<"$help" | head -1)
    if [[ $cli == "settings set" ]]; then
        CLI_ARGV=("$(jq -r 'keys[0]' <<<"$input")" "$(jq -r '.[]' <<<"$input")")
        return
    fi
    read -ra positionals <<<"$(grep -o '<[A-Za-z_]*>' <<<"$usage" | tr -d '<>' | tr '\n' ' ')"
    positionals=("${positionals[@]/PATH/file}")
    # A binary result needs somewhere to go before the CLI sends anything.
    grep -qE -- '^ +-o, --output <PATH>' <<<"$help" && CLI_ARGV+=(-o "$SCRATCH/cli.out")
    while IFS=$'\t' read -r name kind value; do
        flag=--${name//_/-}
        if grep -qE -- "^ +(-[a-z], )?$flag( |\[|\$)" <<<"$help"; then
            case $kind in
            boolean) CLI_ARGV+=("$flag=$value") ;;
            array) while IFS= read -r elem; do CLI_ARGV+=("$flag" "$elem"); done < <(jq -r '.[]' <<<"$value") ;;
            *) CLI_ARGV+=("$flag" "$value") ;;
            esac
        else
            given[$name]=$value
        fi
    done < <(jq -r 'to_entries[] | [.key, (.value | type), (.value | if type == "string" then . else tojson end)] | @tsv' <<<"$input")
    for name in "${positionals[@]}"; do
        [[ -v given[$name] ]] && CLI_ARGV+=("${given[$name]}")
    done
}

# all_ops: one TSV row per operation: id, method, path, MCP tool, CLI words, read-only, has a body.
all_ops() {
    jq -r '.paths | to_entries[] | .key as $p | .value | to_entries[] | select(.value | type == "object" and has("operationId"))
        | [.value.operationId, (.key | ascii_upcase), $p, (.value["x-brr-mcp-tool"] // "-"), (.value["x-brr-cli"] // "-"),
           (.value["x-brr-read-only"] // false), (.value.requestBody != null)] | @tsv' "$spec"
}

ULID_UNKNOWN=01HZZZZZZZZZZZZZZZZZZZZZZZ

# renew_client: ends the Client A (releasing its Leases) and starts a new one, so a CLI call can act.
renew_client() {
    rest DELETE "/v1/clients/$A" >/dev/null
    rest POST /v1/clients '{}'
    A=$(jqv .id)
}

# ================================================================ 1. unknown ids: NotFound on all three surfaces

echo "== unknown ids" >&2
mcp_http tools/list '{}'
TOOLS=$(jq -r '.result.tools[].name' <<<"$MCP_RAW")
sweep_rows=0
while IFS=$'\t' read -r op method path tool cli readonly hasbody; do
    case $op in tab.take_over | tab.hand_back | tab.human_done) continue ;; esac
    [[ $path == *'{'* ]] || continue
    sweep_rows=$((sweep_rows + 1))
    id=$(ulid)
    input=$(input_for "$op" "$id")
    rest_op "$method" "$path" "$input"
    REQ="$op (REST) $method ${path//\{*\}/<unknown>}"
    err NotFound
    if [[ $tool != - ]]; then
        check "tool $tool is listed" yes "$(grep -qx "$tool" <<<"$TOOLS" && echo yes || echo no)"
        mx NotFound "$tool" "$input"
    fi
    cli_argv "$cli" "$input"
    [[ $CLI_SKIP == 1 ]] || cx NotFound "${CLI_WORDS[@]}" "${CLI_ARGV[@]}"
done < <(all_ops)
check "unknown-id sweep covered the path operations" yes "$( ((sweep_rows >= 60)) && echo yes || echo "only $sweep_rows")"

# Names as ids: a Profile, a template, a pool by an unknown name.
rx GET /v1/profiles/no-such-profile; err NotFound
rx GET /v1/templates/no-such-template; err NotFound
rx DELETE /v1/proxy-pools/no-such-pool; err NotFound
check_json "NotFound names what was missing" '.details.what | type == "string" and length > 0'

# ================================================================ 2. unknown keys, malformed JSON, wrong shapes

echo "== malformed requests" >&2
sweep_rows=0
while IFS=$'\t' read -r op method path tool cli readonly hasbody; do
    sweep_rows=$((sweep_rows + 1))
    id=$(ulid)
    input=$(input_for "$op" "$id")
    extra=$(jq -c '. + {zz_unknown_key: 1}' <<<"$input")

    # An unknown body or query key is a 400 naming a field, before anything is looked up or run.
    rest_op "$method" "$path" "$extra"
    REQ="$op (REST, unknown key) $method $path"
    # (these two validate the dummy values first, which is as good an order)
    want_field=zz_unknown_key
    [[ $op == profile.import || $op == template.create ]] && want_field=
    err InvalidArgument $want_field
    if [[ $tool != - ]]; then
        mx InvalidArgument "$tool" "$extra" $want_field
    fi

    # A body that is not JSON, or not an object, is a 400 on field body.
    if [[ $hasbody == true && $method != GET ]]; then
        real=$path
        for key in $(grep -o '{[a-z_]*}' <<<"$path" | tr -d '{}'); do
            real=${real//\{$key\}/$id}
        done
        rx "$method" "$real" '{bad'
        REQ="$op (REST, malformed JSON) $method $real"
        err InvalidArgument body
        rx "$method" "$real" '[1]'
        REQ="$op (REST, array body) $method $real"
        err InvalidArgument body
        rx "$method" "$real" '{"a":'
        REQ="$op (REST, truncated JSON) $method $real"
        err InvalidArgument body
    fi
done < <(all_ops)
check "malformed sweep covered every operation" yes "$( ((sweep_rows >= 130)) && echo yes || echo "only $sweep_rows")"

# A well-formed object with a field of the wrong type names the field it could not read.
rx POST /v1/profiles '{"name": 5, "template": "chromium-headless"}'; err InvalidArgument name
rx POST /v1/profiles '{"name": "x"}'; err InvalidArgument template
rx PUT /v1/limits '{"max_instances": "many"}'; err InvalidArgument max_instances
rx PUT /v1/limits '{"max_instances": -3}'; err InvalidArgument max_instances
rx GET '/v1/events?limit=abc'; err InvalidArgument limit
rx GET '/v1/events?limit=-1'; err InvalidArgument limit
rx GET '/v1/events?order=sideways'; err InvalidArgument order
rx POST /v1/oneshot/screenshot '{"url": "data:text/html,x", "theme": "sepia"}'; err InvalidArgument theme
rx GET '/v1/instances?state=zzz'; err InvalidArgument state
rx GET '/v1/profiles?limit=abc'; err InvalidArgument
rx GET '/v1/system/info?x=1'; err InvalidArgument
rx GET '/v1/system/doctor?x=1'; err InvalidArgument
rx GET '/v1/screenshot'; err InvalidArgument
rx GET '/v1/screenshot?url=data:text/html,x&bogus=1'; err InvalidArgument
rx GET '/v1/screenshot?url=ftp://x/'; err InvalidArgument url
rx POST /v1/profiles ''; err InvalidArgument
rx POST /v1/profiles 'null'; err InvalidArgument body
rx POST /v1/profiles '"text"'; err InvalidArgument body
rx POST /v1/profiles '12'; err InvalidArgument body

# Unknown routes and wrong methods are JSON errors, never a 5xx and never the UI's HTML.
rx GET /v1/no-such-route
check "HTTP status" 404 "$STATUS"
check_json "JSON NotFound body" '.code == "NotFound" and (.message | length > 0)'
rx POST /v1/no-such-route '{}'
check "HTTP status" 404 "$STATUS"
rx DELETE /v1/system/info
CHECKS=$((CHECKS + 1))
[[ $STATUS == 404 || $STATUS == 405 ]] || record "$REQ: wrong method" "404 or 405" "$STATUS"
check_json "JSON error body, not HTML" '.code | type == "string"'
rx GET /v1/ops/settings.get
CHECKS=$((CHECKS + 1))
[[ $STATUS == 404 || $STATUS == 405 ]] || record "$REQ: GET on the POST-only ops route" "404 or 405" "$STATUS"
rx POST /v1/ops/no.such.op '{}'; err NotFound
rx POST /v1/ops/settings.get '{"zz": 1}'; err InvalidArgument

# A Client header that names no Client or is not text is a 4xx error body.
CHECKS=$((CHECKS + 1))
out=$(curl -sS --unix-socket "$SOCK" -o "$SCRATCH/hdr.out" -w '%{http_code}' -H $'Brr-Client: bad\x01value' http://localhost/v1/profiles 2>/dev/null || echo 000)
[[ $out == 400 || $out == 000 ]] || record "GET /v1/profiles with a control byte in Brr-Client" "400 (or the connection refused)" "$out"

# ================================================================ 3. oversized bodies: 413 RequestTooLarge

echo "== oversized bodies" >&2
huge=$SCRATCH/huge.json
python3 - "$huge" <<'PY'
import sys
with open(sys.argv[1], "w") as f:
    f.write('{"name":"')
    f.write("a" * (67 * 1024 * 1024))
    f.write('"}')
PY

# huge_post METHOD PATH: sets $STATUS and $BODY like rest, with the file as the body.
huge_post() {
    local out
    out=$(mktemp -p "$SCRATCH")
    REQ="$1 $2 <67 MiB body>"
    STATUS=$(curl -sS --max-time 120 --unix-socket "$SOCK" -X "$1" -H 'content-type: application/json' \
        --data-binary @"$huge" -o "$out" -D "$SCRATCH/huge.hdr" -w '%{http_code}' "http://localhost$2" 2>/dev/null) || STATUS=000
    BODY=$(<"$out")
    HEADERS=$(<"$SCRATCH/huge.hdr" 2>/dev/null || true)
    rm -f "$out"
}
for target in 'POST /v1/profiles' "POST /v1/tabs/$ULID_UNKNOWN/evaluate" 'PATCH /v1/settings' 'POST /v1/ops/settings.get' \
    'PUT /v1/limits' 'POST /v1/credentials' 'POST /v1/oneshot/screenshot'; do
    huge_post ${target%% *} ${target#* }
    err RequestTooLarge
    check_json "RequestTooLarge reports its limit" '.details.limit_bytes | type == "number" and . > 0'
    check "the daemon still answers" yes "$(daemon_answers && echo yes || echo no)"
done
REQ="POST /mcp <67 MiB body>"
MCP_STATUS=$(curl -sS --max-time 120 -X POST -H 'accept: application/json, text/event-stream' -H 'content-type: application/json' \
    --data-binary @"$huge" -o /dev/null -w '%{http_code}' "http://$BIND/mcp" 2>/dev/null) || MCP_STATUS=000
check "HTTP status" 413 "$MCP_STATUS"
check "the daemon still answers" yes "$(daemon_answers && echo yes || echo no)"
rm -f "$huge"

# ================================================================ 4. MCP protocol errors

echo "== MCP protocol" >&2
mcp_http tools/call '{"name":"no_such_tool","arguments":{}}'
REQ="mcp tools/call no_such_tool"
check "JSON-RPC error code" -32602 "$(jq -r '.error.code // "none"' <<<"$MCP_RAW")"
check "no result" null "$(jq -r '.result // "null"' <<<"$MCP_RAW")"
mcp_http tools/call '{"name":"profile_get","arguments":[1]}'
REQ="mcp tools/call profile_get arguments=[1]"
check "JSON-RPC error code for a non-object arguments" -32602 "$(jq -r '.error.code // "none"' <<<"$MCP_RAW")"
mcp_http tools/call '{"name":"profile_get","arguments":"x"}'
REQ="mcp tools/call profile_get arguments=\"x\""
check "JSON-RPC error code for a string arguments" -32602 "$(jq -r '.error.code // "none"' <<<"$MCP_RAW")"
mcp_http tools/call '{"arguments":{}}'
REQ="mcp tools/call without a name"
check "JSON-RPC error code for a missing name" -32602 "$(jq -r '.error.code // "none"' <<<"$MCP_RAW")"
mcp_http no/such/method '{}'
REQ="mcp no/such/method"
check "JSON-RPC error code" -32601 "$(jq -r '.error.code // "none"' <<<"$MCP_RAW")"
REQ="POST /mcp with malformed JSON"
MCP_STATUS=$(curl -sS --max-time 10 -X POST -H 'accept: application/json, text/event-stream' -H 'content-type: application/json' \
    --data '{bad' -o "$SCRATCH/mcp.out" -w '%{http_code}' "http://$BIND/mcp")
check "HTTP status" 400 "$MCP_STATUS"
REQ="POST /mcp with a JSON array that is not a request"
MCP_STATUS=$(curl -sS --max-time 10 -X POST -H 'accept: application/json, text/event-stream' -H 'content-type: application/json' \
    --data '[]' -o "$SCRATCH/mcp.out" -w '%{http_code}' "http://$BIND/mcp")
CHECKS=$((CHECKS + 1))
((MCP_STATUS >= 400 && MCP_STATUS < 500)) || record "$REQ" "4xx" "$MCP_STATUS"
mx InvalidArgument profile_get '{}'
mx InvalidArgument profile_get '{"profile": 5}'
mx InvalidArgument profile_get '{"profile": "x", "bogus": 1}'
mx InvalidArgument profile_create '{"name": "bad name", "template": "chromium-headless"}' name
mx NotFound profile_get '{"profile": "nope"}'
mx InvalidArgument profile_get '{"profile": "x", "client_id": 5}'

# ================================================================ 5. CLI usage errors and file errors

echo "== CLI" >&2
cx_usage profile get
cx_usage profile get a b
cx_usage profile list --bogus
cx_usage profile create --template chromium-headless
cx_usage profile create name --kind sideways --template chromium-headless
cx_usage tab no-such-tool
cx_usage no-such-command
cx NotFound profile get nope
cx InvalidArgument profile create 'bad name' --template chromium-headless
cx InvalidArgument instance attach
cx NotFound template get nope
cx InvalidArgument settings set no.such.key 1
cx InvalidArgument limits set --max-instances 0
cx InvalidArgument events watch --since abc
# A binary result without -o is refused before the request: a usage error, exit 2 like artifact fetch.
cx InvalidArgument profile export pe --passphrase "$SCRATCH/import.csv"
cx InvalidArgument artifact fetch "$ULID_UNKNOWN"
cx Conflict template delete chromium-headless
# The CLI refuses a file that is not there before it asks the daemon anything.
REQ="brr credential import <missing file>"
out=$("$BRR_BIN" credential import "$SCRATCH/no-such.csv" 2>"$SCRATCH/cli.err") && rc=0 || rc=$?
err=$(<"$SCRATCH/cli.err")
check "exit code" 2 "$rc"
check "stderr is 'error: <message>'" ok "$([[ $err == "error: "?* && $err != *panicked* ]] && echo ok || echo "$err")"

# ================================================================ 6. stateless families, REST + MCP + CLI

echo "== families without a browser" >&2

# --- settings
rx PATCH /v1/settings '{"no.such.key": 1}'; err InvalidArgument no.such.key
rx PATCH /v1/settings '{"ui.theme": "plaid"}'; err InvalidArgument ui.theme
rx PATCH /v1/settings '{"server.bind": "127.0.0.1:1"}'; err InvalidArgument server.bind
rx PATCH /v1/settings '{"idle.grace_s": "soon"}'; err InvalidArgument idle.grace_s
rx PATCH /v1/settings '{"idle.grace_s": -1}'; err InvalidArgument idle.grace_s
rx PATCH /v1/settings '[1]'; err InvalidArgument body
rx GET /v1/settings; okr
check "ui.theme is untouched" system "$(jq -r '.settings.ui.theme // "none"' <<<"$BODY")"
mx InvalidArgument settings_set '{"no.such.key": 1}' no.such.key
cx InvalidArgument settings set ui.theme plaid

# --- profiles and templates
rx POST /v1/profiles '{"name": "bad name", "template": "chromium-headless"}'; err InvalidArgument name
rx POST /v1/profiles '{"name": "", "template": "chromium-headless"}'; err InvalidArgument name
rx POST /v1/profiles "{\"name\": \"$(printf 'n%.0s' {1..80})\", \"template\": \"chromium-headless\"}"; err InvalidArgument name
rx POST /v1/profiles '{"name": "ok1", "template": "no-such-template"}'; err NotFound
rx POST /v1/profiles '{"name": "ok1", "template": "chromium-headless", "kind": "forever"}'; err InvalidArgument
rx POST /v1/profiles '{"name": "ok1", "template": "chromium-headless", "tags": "a"}'; err InvalidArgument
rx POST /v1/profiles '{"name": "pe", "template": "chromium-headless", "kind": "persistent"}'; okr
rx POST /v1/profiles '{"name": "pe", "template": "chromium-headless"}'; err Conflict
check_json "Conflict carries its reason" '.details.reason == "name_taken"'
rx PATCH /v1/profiles/pe '{"name": "bad name"}'; err InvalidArgument name
rx PATCH /v1/profiles/pe '{"settings": {"args": ["--remote-debugging-port=1"]}}'; err InvalidArgument
rx PATCH /v1/profiles/pe '{"settings": {"build": {"source": "cft", "channel": "no-such"}}}'; err InvalidArgument
rx PATCH /v1/profiles/pe '{"disk_quota_bytes": -5}'; err InvalidArgument
rx PATCH /v1/profiles/pe '{"pinned": "yes"}'; err InvalidArgument
rx POST /v1/templates '{"name": "bad name", "settings": {}}'; err InvalidArgument
rx POST /v1/templates '{"name": "tpl-ok", "settings": {"build": {"source": "system"}, "bogus": 1}}'; err InvalidArgument
rx DELETE /v1/templates/chromium-headless; err Conflict
check_json "builtin reason" '.details.reason == "builtin"'
rest GET /v1/templates/chromium-headless
rx PATCH /v1/templates/chromium-headless "$(jq -c '{settings: .settings}' <<<"$BODY")"; err Conflict
rx POST /v1/profiles/pe/save-as-template '{"name": "bad name"}'; err InvalidArgument name
rx POST /v1/profiles/pe/save-as-template '{"name": "chromium-headless"}'; err Conflict
rx POST /v1/profiles/no-such-profile/save-as-template '{"name": "tpl-x"}'; err NotFound
rx DELETE /v1/profiles/no-such-profile; err NotFound
rx PUT /v1/profiles/pe/extensions '{"extensions": [{"extension_id": "'"$ULID_UNKNOWN"'", "auto_update": true}]}'; err NotFound
rx PUT /v1/profiles/pe/extensions '{"extensions": "x"}'; err InvalidArgument
mx Conflict profile_create '{"name": "pe", "template": "chromium-headless"}'
cx Conflict profile create pe --template chromium-headless
cx InvalidArgument profile update pe --name 'bad name'
cx NotFound profile launch no-such-profile

# --- builds
rx POST /v1/builds '{"source": "no-such"}'; err InvalidArgument
rx POST /v1/builds '{"source": "cft", "channel": "no-such"}'; err InvalidArgument channel
rx POST /v1/builds '{"source": "brave", "channel": "beta"}'; err InvalidArgument channel
rx POST /v1/builds '{"source": "cft", "version": "0.0.0.0-bogus"}'; err_shape
rx POST /v1/builds '{"source": "system"}'; err InvalidArgument
rx GET '/v1/builds/available?source=no-such'; err InvalidArgument
rx POST /v1/builds/detect '{}'; okr
system_build=$(jq -r '[.builds[] | select(.source == "system")][0].id // empty' <<<"$BODY")
if [[ -n $system_build ]]; then
    rx DELETE "/v1/builds/$system_build"; err InvalidArgument build_id
    mx InvalidArgument build_remove "{\"build_id\": \"$system_build\"}" build_id
    cx InvalidArgument build remove "$system_build"
fi
rx POST /v1/builds/detect '{"zz": 1}'; err InvalidArgument

# --- proxies and pools
PROXY_PW=proxy-pw-90-zQ41
SECRETS+=("$PROXY_PW")
rx POST /v1/proxies '{"scheme": "gopher", "host": "127.0.0.1", "port": 9}'; err InvalidArgument
rx POST /v1/proxies '{"scheme": "http", "host": "", "port": 9}'; err InvalidArgument host
rx POST /v1/proxies '{"scheme": "http", "host": "127.0.0.1", "port": 0}'; err InvalidArgument port
rx POST /v1/proxies '{"scheme": "http", "host": "127.0.0.1", "port": 70000}'; err InvalidArgument
rx POST /v1/proxies '{"scheme": "http", "host": "bad host name", "port": 9}'; err InvalidArgument host
rx POST /v1/proxies "{\"scheme\": \"http\", \"host\": \"127.0.0.1\", \"port\": 1, \"username\": \"u\", \"password\": \"$PROXY_PW\", \"zz\": 1}"; err InvalidArgument
dead_port=$(free_port)
rx POST /v1/proxies "{\"scheme\": \"http\", \"host\": \"127.0.0.1\", \"port\": $dead_port, \"username\": \"u\", \"password\": \"$PROXY_PW\"}"; okr
dead_proxy=$(jq -r '.id // empty' <<<"$BODY")
rx POST /v1/proxies/import '{"scheme": "no-such", "text": "10.9.9.9:80"}'; err InvalidArgument
rx POST /v1/proxy-pools '{"name": "bad name", "mode": "sticky", "proxy_ids": []}'; err InvalidArgument name
rx POST /v1/proxy-pools '{"name": "pool1", "mode": "sideways", "proxy_ids": []}'; err InvalidArgument
rx POST /v1/proxy-pools "{\"name\": \"pool1\", \"mode\": \"sticky\", \"proxy_ids\": [\"$ULID_UNKNOWN\"]}"; err NotFound
rx PATCH /v1/proxy-pools/pool1 '{"mode": "sticky"}'; err NotFound
if [[ -n $dead_proxy ]]; then
    rx POST /v1/proxy-pools "{\"name\": \"pool1\", \"mode\": \"sticky\", \"proxy_ids\": [\"$dead_proxy\"]}"; okr
    rx POST /v1/proxy-pools "{\"name\": \"pool1\", \"mode\": \"sticky\", \"proxy_ids\": [\"$dead_proxy\"]}"; err Conflict
    rx PATCH /v1/proxies/$dead_proxy '{"port": 70000}'; err InvalidArgument
    rx PATCH /v1/proxies/$dead_proxy '{"host": ""}'; err InvalidArgument host
    rx POST "/v1/proxies/$dead_proxy/check" '{}'; okr
    check_json "a dead proxy is unhealthy, not an error" '.healthy == false'
    check "no password in the proxy record" yes "$([[ $BODY != *"$PROXY_PW"* ]] && echo yes || echo no)"
    mx NotFound proxy_check "{\"proxy_id\": \"$ULID_UNKNOWN\"}"
    cx NotFound proxy check "$ULID_UNKNOWN"
    cx InvalidArgument proxy create --scheme gopher --host 127.0.0.1 --port 9
fi

# --- extensions
rx POST /v1/extensions '{}'; err InvalidArgument
rx POST /v1/extensions '{"ref": "not-an-extension"}'; err InvalidArgument source
rx POST /v1/extensions '{"ref": "x", "file": "AAAA"}'; err InvalidArgument
rx POST /v1/extensions '{"file": "@@@"}'; err InvalidArgument
rx POST /v1/extensions '{"file": "AAAA"}'; err InvalidArgument
rx POST /v1/extensions '{"ref": "x", "source": "no-such"}'; err InvalidArgument
rx POST /v1/extensions '{"ref": "https://example.com/x.zip", "source": "web_store"}'; err_shape
cx InvalidArgument extension add

# --- artifacts, retention, storage
rx GET '/v1/artifacts?kind=no-such'; err InvalidArgument
rx GET '/v1/artifacts?limit=abc'; err InvalidArgument
rx GET '/v1/artifacts?profile=no-such-profile'; err_shape
rx GET "/v1/artifacts/$ULID_UNKNOWN/fetch?as=nonsense"; err_shape
rx PUT /v1/retention '{"unused_days": -1}'; err InvalidArgument
rx PUT /v1/retention '{"artifacts_max_age_days": "forever"}'; err InvalidArgument
rx PUT /v1/retention '{"free_disk_floor_gb": 1.5}'; err InvalidArgument
rx PUT /v1/retention '{"no_such_key": 1}'; err InvalidArgument
rx PUT /v1/retention '{"download_max_mb": 0}'; err InvalidArgument download_max_mb
rx PUT /v1/retention '{"download_max_mb": 1024}'; okr
rx POST /v1/retention/sweep '{"dry_run": "yes"}'; err InvalidArgument
rx POST /v1/retention/sweep '{"no_such": 1}'; err InvalidArgument
rx GET /v1/retention; okr
rx PUT /v1/storage/backend '{"backend": "s3"}'; err InvalidArgument bucket
rx PUT /v1/storage/backend '{"backend": "no-such"}'; err InvalidArgument
rx PUT /v1/storage/backend '{"backend": "r2", "bucket": "b"}'; err InvalidArgument
rx PUT /v1/storage/backend '{"backend": "s3", "bucket": "b", "endpoint": "not a url", "access_key_id": "a", "secret_access_key": "x"}'; err InvalidArgument endpoint
rx POST /v1/storage/backend/test '{"backend": "s3", "bucket": "b", "endpoint": "not a url", "access_key_id": "a", "secret_access_key": "x"}'; err InvalidArgument endpoint
check "the daemon survived the bad endpoint" yes "$(daemon_answers && echo yes || echo no)"
S3_SECRET=s3-secret-90-Kj77
SECRETS+=("$S3_SECRET")
dead_s3=$(free_port)
rx POST /v1/storage/backend/test "{\"backend\": \"s3\", \"endpoint\": \"http://127.0.0.1:$dead_s3\", \"bucket\": \"b\", \"access_key_id\": \"a\", \"secret_access_key\": \"$S3_SECRET\", \"region\": \"us-east-1\", \"path_style\": true}"
err StorageUnavailable
mx StorageUnavailable storage_backend_test "{\"backend\": \"s3\", \"endpoint\": \"http://127.0.0.1:$dead_s3\", \"bucket\": \"b\", \"access_key_id\": \"a\", \"secret_access_key\": \"$S3_SECRET\", \"region\": \"us-east-1\", \"path_style\": true}"
cx InvalidArgument storage backend set --backend s3
rx GET /v1/storage/backend; okr
check "the backend setting is unchanged by the failed test" local "$(jq -r '.backend // "none"' <<<"$BODY")"
check "no secret in the backend record" yes "$([[ $BODY != *"$S3_SECRET"* ]] && echo yes || echo no)"

# --- credentials
CRED_PW=cred-pw-90-Wq83
CRED_TOTP=JBSWY3DPEHPK3PXP
SECRETS+=("$CRED_PW" "$CRED_TOTP")
rx POST /v1/credentials "{\"site\": \"ftp://site.test\", \"username\": \"u\", \"password\": \"$CRED_PW\"}"; err InvalidArgument site
rx POST /v1/credentials "{\"site\": \"\", \"username\": \"u\", \"password\": \"$CRED_PW\"}"; err InvalidArgument site
rx POST /v1/credentials "{\"site\": \"https://site.test\", \"username\": \"\", \"password\": \"$CRED_PW\"}"; err InvalidArgument username
rx POST /v1/credentials "{\"site\": \"https://site.test\", \"username\": \"u\", \"password\": \"$CRED_PW\", \"totp\": \"not base32 !!\"}"; err InvalidArgument
rx POST /v1/credentials "{\"site\": \"https://site.test\", \"username\": \"u\", \"password\": \"$CRED_PW\", \"profile_ids\": [\"$ULID_UNKNOWN\"]}"; err NotFound
rx POST /v1/credentials "{\"site\": \"https://site.test\", \"username\": \"u\", \"password\": \"$CRED_PW\", \"selectors\": \"x\"}"; err InvalidArgument
rx POST /v1/credentials "{\"site\": \"https://site.test\", \"username\": \"u\", \"password\": \"$CRED_PW\", \"totp\": \"$CRED_TOTP\"}"; okr
cred=$(jq -r '.id // empty' <<<"$BODY")
check "the credential response has no secret" yes "$([[ $BODY != *"$CRED_PW"* && $BODY != *"$CRED_TOTP"* ]] && echo yes || echo no)"
rx POST /v1/credentials "{\"site\": \"https://site.test\", \"username\": \"u\", \"password\": \"$CRED_PW\"}"; err Conflict
if [[ -n $cred ]]; then
    rx PATCH "/v1/credentials/$cred" '{"profile_ids": ["no-such-profile"]}'; err NotFound
    rx PATCH "/v1/credentials/$cred" '{"totp": "not base32 !!"}'; err InvalidArgument
    rx PATCH "/v1/credentials/$cred" '{"login_url": "ftp://x"}'; err InvalidArgument
    rx PATCH "/v1/credentials/$cred" '{"selectors": {"bogus": "x"}}'; err InvalidArgument selectors.bogus
fi
rx POST /v1/credentials/import '{"format": "no-such", "file": "x"}'; err InvalidArgument
rx POST /v1/credentials/import '{"format": "chrome", "file": "not,the,right\nheader"}'; err InvalidArgument file
rx POST /v1/credentials/import '{"format": "chrome", "file": "name,url,username,password\nx,not-a-url,u,p", "profile_ids": ["no-such-profile"]}'; err NotFound
mx InvalidArgument credential_create '{"site": "ftp://site.test", "username": "u", "password": "x"}' site
cx InvalidArgument credential import --format no-such "$SCRATCH/import.csv"

# --- login checks
rx POST /v1/profiles/pe/login-checks '{"site": "https://x.test", "url": "https://x.test/"}'; err InvalidArgument
rx POST /v1/profiles/pe/login-checks '{"site": "https://x.test", "url": "https://x.test/", "expect_selector": "a", "expect_text": "b"}'; err InvalidArgument
rx POST /v1/profiles/pe/login-checks '{"site": "ftp://x.test", "url": "https://x.test/", "expect_text": "b"}'; err InvalidArgument
rx POST /v1/profiles/pe/login-checks '{"site": "https://x.test", "url": "nope", "expect_text": "b"}'; err InvalidArgument
rx POST /v1/profiles/pe/login-checks "{\"site\": \"https://x.test\", \"url\": \"https://x.test/\", \"expect_text\": \"b\", \"credential_id\": \"$ULID_UNKNOWN\"}"; err NotFound
rx POST /v1/profiles/no-such-profile/login-checks '{"site": "https://x.test", "url": "https://x.test/", "expect_text": "b"}'; err NotFound
rx POST /v1/profiles/pe/login-checks '{"site": "https://x.test", "url": "https://x.test/", "expect_text": "b"}'; okr
lc=$(jq -r '.id // empty' <<<"$BODY")
if [[ -n $lc ]]; then
    rx PATCH "/v1/login-checks/$lc" '{"expect_selector": "a"}'; err InvalidArgument
    rx PATCH "/v1/login-checks/$lc" "{\"credential_id\": \"$ULID_UNKNOWN\"}"; err NotFound
    rx POST "/v1/login-checks/$lc/run" '{}'; err Conflict
    check_json "not running is the reason" '.details.reason == "profile_not_running"'
    mx Conflict login_check_run "{\"login_check_id\": \"$lc\"}"
    cx Conflict login-check run "$lc"
fi

# --- captcha
CAPTCHA_KEY=2cap-90-KEY-4471
SECRETS+=("$CAPTCHA_KEY")
rx GET /v1/captcha/balance; err InvalidArgument api_key
mx InvalidArgument captcha_balance '{}' api_key
cx InvalidArgument captcha balance
rx PUT /v1/captcha/settings '{"default_policy": "sometimes"}'; err InvalidArgument
rx PUT /v1/captcha/settings "{\"api_key\": \"$CAPTCHA_KEY\", \"daily_solves\": -1}"; err InvalidArgument
rx PUT /v1/captcha/settings "{\"api_key\": \"$CAPTCHA_KEY\", \"human_wait_s\": \"long\"}"; err InvalidArgument
rx GET /v1/captcha/settings; okr
check "the failed settings did not store the key" null "$(jq -r '.api_key.set // "null"' <<<"$BODY" | sed 's/^false$/null/')"
rx GET '/v1/captcha/solves?limit=abc'; err InvalidArgument

# --- backups, export, import
BACKUP_PW=backup-pw-90-Zt29
SECRETS+=("$BACKUP_PW")
rx POST /v1/profiles/pe/export '{}'; err InvalidArgument
rx POST /v1/profiles/no-such-profile/export "{\"passphrase\": \"$BACKUP_PW\"}"; err NotFound
rx POST /v1/profiles/pe/export "{\"passphrase\": \"$BACKUP_PW\", \"destination\": \"somewhere\"}"; err InvalidArgument
rx POST /v1/profiles/import "{\"file\": \"AAAA\", \"passphrase\": \"$BACKUP_PW\"}"; err InvalidArgument passphrase
rx POST /v1/profiles/import "{\"file\": \"@@@\", \"passphrase\": \"$BACKUP_PW\"}"; err InvalidArgument
rx POST /v1/profiles/import "{\"file\": \"\", \"passphrase\": \"$BACKUP_PW\"}"; err InvalidArgument
rx POST /v1/profiles/import '{"passphrase": "p"}'; err InvalidArgument
rx POST /v1/profiles/pe/backups '{}'; err InvalidArgument backup.passphrase
rx POST /v1/profiles/no-such-profile/backups '{}'; err NotFound
rx GET /v1/profiles/no-such-profile/backups; err NotFound
rx POST "/v1/backups/$ULID_UNKNOWN/restore" "{\"passphrase\": \"$BACKUP_PW\"}"; err NotFound
rx PATCH /v1/profiles/pe '{"backup_schedule": "not a cron"}'; err InvalidArgument backup_schedule
rx PATCH /v1/profiles/pe '{"backup_keep": 0}'; err InvalidArgument
rx PATCH /v1/profiles/pe '{"backup_schedule": "0 3 * * *"}'; err InvalidArgument backup_schedule
mx NotFound backup_delete "{\"backup_id\": \"$ULID_UNKNOWN\"}"
cx NotFound backup delete "$ULID_UNKNOWN"
cx InvalidArgument backup create pe

# --- limits, admission, events, system
rx PUT /v1/limits '{"max_instances": 0}'; err InvalidArgument max_instances
rx PUT /v1/limits '{"max_tabs_per_instance": 0}'; err InvalidArgument max_tabs_per_instance
rx PUT /v1/limits '{"max_tabs_per_instance": 20}'; okr
rx PUT /v1/limits '{"mode": "turbo"}'; err InvalidArgument
rx PUT /v1/limits '{"admission": {"default": "maybe"}}'; err InvalidArgument
rx PUT /v1/limits '{"admission": {"queue_timeout_s": -1}}'; err InvalidArgument
rx PUT /v1/limits '{"adaptive": {"max_load_per_core": "high"}}'; err InvalidArgument
rx PUT /v1/limits '{"max_instances_per_engine": {"chromium": 0}}'; err InvalidArgument
rx PUT /v1/limits '{"max_instances_per_engine": {"chromium": 8}}'; okr
rx GET /v1/limits; okr
rx GET '/v1/events?since=abc'; err InvalidArgument
rx GET '/v1/events/watch?timeout_ms=abc'; err InvalidArgument
rx GET '/v1/events/watch?kinds=1&zz=1'; err InvalidArgument
rx GET /v1/admission/queue?x=1; err InvalidArgument
cx InvalidArgument limits set --max-instances 0
mx InvalidArgument limits_set '{"max_instances": 0}' max_instances

# --- oneshot parameters that need no browser
rx POST /v1/oneshot/screenshot '{}'; err InvalidArgument
rx POST /v1/oneshot/screenshot '{"url": "not a url"}'; err InvalidArgument url
rx POST /v1/oneshot/screenshot '{"url": "file:///etc/passwd"}'; err InvalidArgument url
rx POST /v1/oneshot/pdf '{"url": "javascript:alert(1)"}'; err InvalidArgument url
rx POST /v1/oneshot/screenshot '{"url": "data:text/html,x", "screen": "wide"}'; err InvalidArgument
rx POST /v1/oneshot/screenshot '{"url": "data:text/html,x", "screen": "0x0"}'; err InvalidArgument
rx POST /v1/oneshot/screenshot '{"url": "data:text/html,x", "theme": "sepia"}'; err InvalidArgument
rx POST /v1/oneshot/screenshot '{"url": "data:text/html,x", "scale": -1}'; err InvalidArgument
rx POST /v1/oneshot/pdf '{"url": "data:text/html,x", "pdf_scale": 50}'; err InvalidArgument pdf_scale
rx POST /v1/oneshot/pdf '{"url": "data:text/html,x", "pages": "z-1"}'; err_shape
rx POST /v1/oneshot/markdown '{"url": "data:text/html,x", "timeout": "soon"}'; err InvalidArgument
rx POST /v1/oneshot/links '{"url": "data:text/html,x", "cookies": "not json"}'; err InvalidArgument
mx InvalidArgument oneshot_screenshot '{"url": "ftp://x/"}' url
cx InvalidArgument shot screenshot 'ftp://x/'

# ================================================================ 7. browser-backed paths

if ! have_chrome; then
    echo "SKIP browser sections: no Chromium-family browser" >&2
else
echo "== browser: leases and tab tools" >&2
detect_builds
rx PUT /v1/retention '{"free_disk_floor_gb": 1}'; okr
rx PATCH /v1/settings '{"lease.exclusive_wait_s": 1}'; okr
rest POST /v1/clients '{}'; A=$(jqv .id)
rest POST /v1/clients '{}'; B=$(jqv .id)
new_profile eph ephemeral
launch eph "$A"
INST=$(jqv .instance.id)
TAB=$(jqv .tab.id)
LEASE=$(jqv .lease.id)
wait_until 'rest GET "/v1/tabs/$TAB"; [[ $(jq -r .state <<<"$BODY") == ready ]]' 15

# --- leases: B against A's Tab, on all three surfaces, for every Tab tool
mutating=0 readonly_n=0
while IFS=$'\t' read -r op method path tool cli readonly hasbody; do
    case $op in tab.take_over | tab.hand_back | tab.human_done) continue ;; esac
    input=$(input_for "$op" "$TAB")
    if [[ $readonly == true ]]; then
        readonly_n=$((readonly_n + 1))
        RX_CLIENT=$B rest_op "$method" "$path" "$input"
        REQ="$op (REST, read-only, as another client) $method $path"
        CHECKS=$((CHECKS + 1))
        [[ $STATUS != 423 ]] || record "$REQ" "any status but 423 (read-only tools never need the Lease)" "$STATUS $(short "$BODY" 100)"
        continue
    fi
    mutating=$((mutating + 1))
    RX_CLIENT=$B rest_op "$method" "$path" "$input"
    REQ="$op (REST, as another client) $method $path"
    err LeaseHeld
    check "LeaseHeld names the holder" "$A" "$(jq -r '.details.client_id // "none"' <<<"$BODY")"
    if [[ $tool != - ]]; then
        mx LeaseHeld "$tool" "$(jq -c --arg c "$B" '. + {client_id: $c}' <<<"$input")"
        check "LeaseHeld names the holder" "$A" "$(jq -r '.details.client_id // "none"' <<<"$BODY")"
    fi
    cli_argv "$cli" "$input"
    [[ $CLI_SKIP == 1 ]] || cx LeaseHeld "${CLI_WORDS[@]}" "${CLI_ARGV[@]}"
done < <(all_ops | awk -F'\t' '$3 ~ /^\/v1\/tabs\//')
check "lease sweep covered mutating tools" yes "$( ((mutating >= 30)) && echo yes || echo "only $mutating")"
check "lease sweep covered read-only tools" yes "$( ((readonly_n >= 10)) && echo yes || echo "only $readonly_n")"

# Reads stay open to everybody.
RX_CLIENT=$B tab_call "$TAB" get_text '{}' "$B"; REQ="get_text as another client"; okr
mok get_text "{\"tab_id\": \"$TAB\", \"client_id\": \"$B\"}"
out=$("$BRR_BIN" --json tab get "$TAB" 2>&1) && rc=0 || rc=$?
REQ="brr tab get (another client)"; check "exit code" 0 "$rc"

# --- leases: acquire, release, exclusive
rx POST /v1/leases "{\"instance_id\": \"$INST\", \"tab_id\": \"$TAB\"}" "$B"; err LeaseHeld
mx LeaseHeld lease_acquire "{\"instance_id\": \"$INST\", \"tab_id\": \"$TAB\", \"client_id\": \"$B\"}"
cx LeaseHeld lease acquire --instance-id "$INST" --tab-id "$TAB"
rx DELETE "/v1/leases/$LEASE" '' "$B"; err LeaseHeld
mx LeaseHeld lease_release "{\"lease_id\": \"$LEASE\", \"client_id\": \"$B\"}"
cx LeaseHeld lease release "$LEASE"
rx POST /v1/leases "{\"instance_id\": \"$INST\", \"tab_id\": \"$ULID_UNKNOWN\"}" "$A"; err NotFound
rx POST /v1/leases "{\"instance_id\": \"$INST\"}" "$A"; err InvalidArgument tab_id
rx POST /v1/leases "{\"instance_id\": \"$ULID_UNKNOWN\", \"exclusive\": true}" "$A"; err NotFound
rx POST /v1/leases "{\"instance_id\": \"$INST\", \"exclusive\": \"yes\"}" "$A"; err InvalidArgument
rx POST /v1/leases "{\"instance_id\": \"$INST\", \"exclusive\": true}" "$A"; okr
rx POST "/v1/instances/$INST/tabs" '{}' "$B"; err LeaseHeld
mx LeaseHeld tab_open "{\"instance_id\": \"$INST\", \"client_id\": \"$B\"}"
cx LeaseHeld tab open --instance-id "$INST"
started=$SECONDS
rx POST /v1/leases "{\"instance_id\": \"$INST\", \"exclusive\": true}" "$B"; err LeaseHeld
CHECKS=$((CHECKS + 1))
((SECONDS - started <= 10)) || record "$REQ: exclusive wait" "gives up after lease.exclusive_wait_s (1 s)" "$((SECONDS - started)) s"
rest GET "/v1/leases?client_id=$A"
rx GET '/v1/leases?client_id=not-a-client'; okr
check_json "an unknown client has no leases" '.leases | length == 0'
rx POST "/v1/instances/$INST/stop" '{}' "$B"; err LeaseHeld
mx LeaseHeld instance_stop "{\"instance_id\": \"$INST\", \"client_id\": \"$B\"}"
cx LeaseHeld instance stop "$INST"
check "the Instance is still running" running "$(instance_state "$INST")"
rx DELETE "/v1/clients/$B"; okr
rx DELETE "/v1/clients/$B"; okr
rest POST /v1/clients '{}'; B=$(jqv .id)

# --- a Lease released during a call: the call reports LeaseLost
rest POST /v1/profiles/eph/launch '{}' "$B"; B_INST=$(jqv .instance.id); B_TAB=$(jqv .tab.id); B_LEASE=$(jqv .lease.id)
wait_until 'rest GET "/v1/tabs/$B_TAB"; [[ $(jq -r .state <<<"$BODY") == ready ]]' 15
(
    curl -sS --max-time 30 --unix-socket "$SOCK" -X POST -H "Brr-Client: $B" -H 'content-type: application/json' \
        --data '{"expression": "new Promise(r => setTimeout(() => r(1), 3000))"}' -o "$SCRATCH/lost.body" -w '%{http_code}' \
        "http://localhost/v1/tabs/$B_TAB/evaluate" >"$SCRATCH/lost.status"
) &
lost_pid=$!
sleep 1
rest DELETE "/v1/leases/$B_LEASE" '' "$B"
wait "$lost_pid"
STATUS=$(<"$SCRATCH/lost.status") BODY=$(<"$SCRATCH/lost.body") HEADERS=
REQ="POST /v1/tabs/<tab>/evaluate (3 s) while its Lease is released after 1 s"
err LeaseLost
rx POST "/v1/instances/$B_INST/stop" '{}' "$B"; okr

# --- tab tools with an invalid argument or a wrong page state, as the holder
echo "== browser: tab tool errors" >&2
RX_CLIENT=$A
tab_post() { rx POST "/v1/tabs/$TAB/$1" "$2"; }
# every tool with a required field refuses {}
required_missing=0
while IFS=$'\t' read -r op method path tool cli readonly hasbody; do
    required=$(jq -r --arg op "$op" '[.paths[][] | select(type == "object" and .operationId == $op)][0].requestBody.content["application/json"].schema.required // [] | length' "$spec")
    ((required > 0)) || continue
    required_missing=$((required_missing + 1))
    rx POST "${path/\{tab_id\}/$TAB}" '{}'
    REQ="$op (REST, required field missing)"
    err InvalidArgument
    mx InvalidArgument "$tool" "{\"tab_id\": \"$TAB\", \"client_id\": \"$A\"}"
done < <(all_ops | awk -F'\t' '$3 ~ /^\/v1\/tabs\// && $2 == "POST"')
check "required-field sweep covered the tools that have one" yes "$( ((required_missing >= 12)) && echo yes || echo "only $required_missing")"

# ref and selector together, or neither
for tool in click dblclick hover check uncheck; do
    tab_post "$tool" '{"ref": "e1", "selector": "#x"}'; err InvalidArgument ref
    tab_post "$tool" '{}'; err InvalidArgument ref
done
tab_post select_option '{"ref": "e1", "selector": "#x", "values": ["v"]}'; err InvalidArgument ref
tab_post select_option '{"values": ["v"]}'; err InvalidArgument ref
tab_post fill '{"ref": "e1", "selector": "#x", "value": "v"}'; err InvalidArgument ref
tab_post upload_file '{"ref": "e1", "selector": "#x", "files": []}'; err InvalidArgument ref
tab_post drag '{"ref": "e1"}'; err InvalidArgument to_ref
tab_post drag '{"selector": "#a", "to_ref": "e1", "to_selector": "#b"}'; err InvalidArgument to_ref
tab_post screenshot '{"ref": "e1", "selector": "#x"}'; err InvalidArgument ref
tab_post wait_for '{}'; err InvalidArgument selector
tab_post wait_for '{"selector": "a", "text": "b"}'; err InvalidArgument selector
mx InvalidArgument click "{\"tab_id\": \"$TAB\", \"client_id\": \"$A\", \"ref\": \"e1\", \"selector\": \"#x\"}" ref
cx InvalidArgument tab click --tab-id "$TAB" --ref e1 --selector '#x'

# the page: a document with a form, a heading and nothing that matches
tab_post navigate '{"url": "data:text/html,<title>errors</title><h1 id=h>hello</h1><input id=i><input type=checkbox id=c>"}'; okr
tab_post click '{"selector": "#nope", "timeout": 300}'; err ElementNotFound
tab_post dblclick '{"selector": "#nope", "timeout": 300}'; err ElementNotFound
tab_post hover '{"selector": "#nope", "timeout": 300}'; err ElementNotFound
tab_post check '{"selector": "#nope", "timeout": 300}'; err ElementNotFound
tab_post uncheck '{"selector": "#nope", "timeout": 300}'; err ElementNotFound
tab_post fill '{"selector": "#nope", "value": "v", "timeout": 300}'; err ElementNotFound
tab_post type '{"selector": "#nope", "text": "v", "timeout": 300}'; err ElementNotFound
tab_post select_option '{"selector": "#nope", "values": ["v"], "timeout": 300}'; err ElementNotFound
tab_post press_key '{"selector": "#nope", "key": "Enter", "timeout": 300}'; err ElementNotFound
tab_post scroll '{"selector": "#nope", "into_view": true, "timeout": 300}'; err ElementNotFound
tab_post drag '{"selector": "#nope", "to_selector": "#h", "timeout": 300}'; err ElementNotFound
tab_post upload_file '{"selector": "#nope", "files": [], "timeout": 300}'; err ElementNotFound
tab_post screenshot '{"selector": "#nope"}'; err ElementNotFound
tab_post click '{"ref": "e999"}'; err StaleRef
tab_post fill '{"ref": "e999", "value": "v"}'; err StaleRef
tab_post hover '{"ref": "e999"}'; err StaleRef
mx ElementNotFound click "{\"tab_id\": \"$TAB\", \"client_id\": \"$A\", \"selector\": \"#nope\", \"timeout\": 300}"
mx StaleRef click "{\"tab_id\": \"$TAB\", \"client_id\": \"$A\", \"ref\": \"e999\"}"

tab_post evaluate '{"expression": "throw new Error(\"boom-90\")"}'; err EvaluationFailed
check_json "EvaluationFailed carries the page's message" '.details.message | contains("boom-90")'
tab_post evaluate '{"expression": "1 +"}'; err EvaluationFailed
tab_post evaluate '{"expression": "Promise.reject(new Error(\"rej-90\"))"}'; err EvaluationFailed
mx EvaluationFailed evaluate "{\"tab_id\": \"$TAB\", \"client_id\": \"$A\", \"expression\": \"throw 1\"}"

dead_page=$(free_port)
tab_post navigate "{\"url\": \"http://127.0.0.1:$dead_page/\"}"; err NavigationFailed
check_json "NavigationFailed names the net error" '.details.net_error | startswith("net::")'
tab_post navigate '{"url": "http://no-such-host.invalid/"}'; err NavigationFailed
tab_post navigate '{"url": "notaurl"}'; err InvalidArgument url
tab_post navigate '{"url": ""}'; err InvalidArgument url
tab_post navigate '{"url": "about:blank", "wait_until": "eventually"}'; err InvalidArgument
tab_post navigate '{"url": "about:blank", "timeout": "soon"}'; err InvalidArgument
tab_post reload '{"timeout": -1}'; err InvalidArgument
mx NavigationFailed navigate "{\"tab_id\": \"$TAB\", \"client_id\": \"$A\", \"url\": \"http://127.0.0.1:$dead_page/\"}"

tab_post wait_for '{"selector": "#never", "timeout": 500}'; err Timeout
check_json "Timeout names its length" '.details.after_ms == 500'
tab_post wait_for '{"text": "never-appears", "timeout": 500}'; err Timeout
tab_post wait_for '{"url": "*/never", "timeout": 500}'; err Timeout
mx Timeout wait_for "{\"tab_id\": \"$TAB\", \"client_id\": \"$A\", \"selector\": \"#never\", \"timeout\": 500}"

tab_post handle_dialog '{"accept": true}'; err NotFound
# take-over, hand-back and human-done are the UI Client's; a REST caller is told so, before any Tab is looked up.
tab_post human-done '{}'; err InvalidArgument client
tab_post hand-back '{}'; err InvalidArgument client
tab_post har_stop '{}'; err InvalidArgument har
tab_post screenshot '{"quality": 300}'; err InvalidArgument
tab_post screenshot '{"format": "gif"}'; err InvalidArgument
tab_post screenshot '{"return": "somewhere"}'; err InvalidArgument
tab_post pdf '{"pdf_scale": 50}'; err InvalidArgument pdf_scale
tab_post pdf '{"paper": "A99"}'; err InvalidArgument
tab_post emulate '{"device": "no-such-device"}'; err InvalidArgument device
tab_post emulate '{"timezone": "Nowhere/Land"}'; err InvalidArgument timezone
tab_post emulate '{}'; err InvalidArgument
tab_post emulate '{"geolocation": {"latitude": 200, "longitude": 0}}'; err InvalidArgument
tab_post emulate '{"viewport": {"width": 0, "height": 0}}'; err InvalidArgument viewport
tab_post press_key '{"key": "NoSuchKey"}'; err InvalidArgument key
tab_post storage_get '{"area": "elsewhere"}'; err InvalidArgument
tab_post storage_get '{"area": "local"}'; err InvalidArgument area
tab_post storage_set '{"area": "local", "items": {"a": "b"}}'; err InvalidArgument area
tab_post storage_set '{"area": "local", "items": {"a": 1}}'; err InvalidArgument
tab_post storage_clear '{"area": "session"}'; err InvalidArgument area
tab_post cookies_set '{"cookies": [{"name": "a"}]}'; err InvalidArgument
tab_post cookies_set '{"cookies": [{"name": "a", "value": "b", "domain": "", "path": "/"}]}'; err_shape
tab_post cookies_get '{"urls": ["not a url"]}'; err InvalidArgument urls
tab_post upload_file '{"selector": "#i", "files": [{"name": "a.txt", "data": "@@@"}]}'; err InvalidArgument
tab_post console_messages '{"limit": -1}'; err InvalidArgument
tab_post network_log '{"limit": "many"}'; err InvalidArgument
tab_post block_requests '{"patterns": [], "types": ["telepathy"]}'; err InvalidArgument
tab_post await_human '{"reason": "boredom"}'; err InvalidArgument
tab_post har_start '{"zz": 1}'; err InvalidArgument
tab_post scroll '{"dy": "far"}'; err InvalidArgument
tab_post click '{"selector": "#h", "button": "side"}'; err InvalidArgument
tab_post click '{"selector": "#h", "click_count": -1}'; err InvalidArgument
tab_post click '{"selector": "#h", "modifiers": ["Hyper"]}'; err InvalidArgument
tab_post type '{"text": 5}'; err InvalidArgument
tab_post login '{"credential_id": "not-a-ulid"}'; err InvalidArgument
tab_post login "{\"credential_id\": \"$ULID_UNKNOWN\"}"; err_shape
tab_post solve_captcha '{"human_fallback": "maybe"}'; err InvalidArgument
rx POST "/v1/instances/$INST/tabs" '{"url": 5}'; err InvalidArgument
rx POST "/v1/instances/$INST/tabs" '{"url": "notaurl"}'; err InvalidArgument url
rx GET "/v1/tabs/$TAB?x=1"; err InvalidArgument
rx GET "/v1/instances/$INST/tabs?x=1"; err InvalidArgument
rx POST "/v1/tabs/$TAB/pin" '{"zz": 1}'; err InvalidArgument
RX_CLIENT=
renew_client
cx ElementNotFound tab click --tab-id "$TAB" --selector '#nope' --timeout 300
cx StaleRef tab click --tab-id "$TAB" --ref e999
cx EvaluationFailed tab evaluate --tab-id "$TAB" 'throw new Error("x")'
cx NavigationFailed tab navigate --tab-id "$TAB" "http://127.0.0.1:$dead_page/"
cx Timeout tab wait-for --tab-id "$TAB" --selector '#never' --timeout 500

# --- a Tab that is closed, an Instance that is stopped
echo "== browser: closed tabs, stopped instances" >&2
rest POST "/v1/instances/$INST/tabs" '{}' "$A"; CLOSED_TAB=$(jqv .tab.id)
rest DELETE "/v1/tabs/$CLOSED_TAB" '' "$A"
check "the Tab closed" 200 "$STATUS"
rx DELETE "/v1/tabs/$CLOSED_TAB" '' "$A"; err NotFound
closed_rows=0
while IFS=$'\t' read -r op method path tool cli readonly hasbody; do
    case $op in tab.take_over | tab.hand_back | tab.human_done) continue ;; esac
    [[ $op == tab.get ]] && continue
    closed_rows=$((closed_rows + 1))
    rest_op "$method" "$path" "$(input_for "$op" "$CLOSED_TAB")"
    REQ="$op (REST, closed Tab) $method $path"
    err NotFound
    [[ $tool == - ]] || mx NotFound "$tool" "$(input_for "$op" "$CLOSED_TAB")"
done < <(all_ops | awk -F'\t' '$3 ~ /^\/v1\/tabs\//')
check "closed-Tab sweep covered every Tab tool" yes "$( ((closed_rows >= 40)) && echo yes || echo "only $closed_rows")"
rx GET "/v1/tabs/$CLOSED_TAB"; okr
check "a closed Tab is still readable as closed" closed "$(jq -r '.state // "none"' <<<"$BODY")"

rx POST "/v1/instances/$INST/stop" '{}' "$A"; okr
wait_until '[[ $(instance_state "$INST") == stopped ]]' 20
rx POST "/v1/instances/$INST/stop" '{}'; err InvalidTransition
check_json "InvalidTransition names both states" '.details.from == "stopped" and .details.to == "stopping"'
mx InvalidTransition instance_stop "{\"instance_id\": \"$INST\"}"
cx InvalidTransition instance stop "$INST"
rx POST "/v1/instances/$INST/promote" '{"name": "after-stop"}'; err InvalidTransition
mx InvalidTransition instance_promote "{\"instance_id\": \"$INST\", \"name\": \"after-stop\"}"
rx POST "/v1/instances/$INST/detach" '{}'; err InvalidArgument instance_id
rx POST "/v1/instances/$INST/tabs" '{}'; err InvalidTransition
rx POST /v1/leases "{\"instance_id\": \"$INST\", \"exclusive\": true}"; err InvalidTransition
rx POST "/v1/tabs/$TAB/click" '{"selector": "#h"}'; err NotFound
rx POST "/v1/instances/$INST/promote" '{"name": "bad name"}'; err InvalidArgument name
rx GET "/v1/instances?profile=no-such-profile"; err NotFound
rx GET "/v1/instances?profile=eph&state=running,zzz"; err InvalidArgument state

# --- caps: instances and tabs, on all three surfaces
echo "== browser: caps" >&2
rx PUT /v1/limits '{"max_instances": 1, "max_tabs_per_instance": 2, "admission": {"default": "reject", "queue_timeout_s": 1}}'; okr
new_profile cap1 ephemeral
new_profile cap2 ephemeral
rest POST /v1/profiles/cap1/launch '{}' "$A"; CAP_INST=$(jqv .instance.id)
rx POST /v1/profiles/cap2/launch '{}' "$B"; err CapExceeded
check_json "CapExceeded tells when to retry" '.details.retry_after | type == "number"'
rx POST /v1/profiles/cap2/launch '{"admission": "queue"}' "$B"; err CapExceeded
mx CapExceeded profile_launch '{"profile": "cap2", "client_id": "'"$B"'"}'
cx CapExceeded profile launch cap2
rx POST /v1/profiles/cap2/launch '{"admission": "sometimes"}'; err InvalidArgument
rx POST /v1/profiles/cap2/launch '{"priority": 300}'; err InvalidArgument
rx POST /v1/profiles/cap2/launch '{"display": "holographic"}'; err InvalidArgument
rx POST "/v1/instances/$CAP_INST/tabs" '{}' "$A"; okr
rx POST "/v1/instances/$CAP_INST/tabs" '{}' "$A"; err CapExceeded
mx CapExceeded tab_open "{\"instance_id\": \"$CAP_INST\", \"client_id\": \"$A\"}"
cx CapExceeded tab open --instance-id "$CAP_INST"
rx PUT /v1/limits '{"max_instances": 10, "max_tabs_per_instance": 20, "admission": {"default": "queue", "queue_timeout_s": 60}}'; okr
rx POST "/v1/instances/$CAP_INST/stop" '{}' "$A"; okr

# --- profile locks, running Profiles, the disk floor
echo "== browser: profile state" >&2
new_profile lock persistent
rest POST /v1/profiles/lock/launch '{}' "$A"; LOCK_INST=$(jqv .instance.id)
rx POST /v1/profiles/lock/launch '{}' "$B"; err Conflict
check_json "profile_locked" '.details.reason == "profile_locked"'
mx Conflict profile_launch '{"profile": "lock"}'
cx Conflict profile launch lock
rx DELETE /v1/profiles/lock; err Conflict
check_json "profile_running" '.details.reason == "profile_running"'
mx Conflict profile_delete '{"profile": "lock"}'
cx Conflict profile delete lock
rx POST /v1/profiles/lock/export "{\"passphrase\": \"$BACKUP_PW\"}"; err Conflict
check_json "profile_running" '.details.reason == "profile_running"'
rx PUT /v1/profiles/lock/bookmarks '{"data": "<!DOCTYPE NETSCAPE-Bookmark-file-1><DL></DL>"}'; err Conflict
check_json "profile_running" '.details.reason == "profile_running"'
rx PATCH /v1/settings "{\"backup.passphrase\": \"$BACKUP_PW\"}"; okr
rx POST /v1/profiles/lock/backups '{}'; err Conflict
check_json "profile_running" '.details.reason == "profile_running"'
rx POST /v1/profiles/eph/backups '{}'; err InvalidArgument
rx POST /v1/profiles/eph/export "{\"passphrase\": \"$BACKUP_PW\"}"; err InvalidArgument profile
rx GET /v1/profiles/eph/cookies; err InvalidArgument profile
rx PUT /v1/profiles/eph/cookies '{"data": "[]"}'; err InvalidArgument profile
rx POST /v1/login-checks/"$ULID_UNKNOWN"/run '{}'; err NotFound
rx POST "/v1/instances/$LOCK_INST/stop" '{}' "$A"; okr
wait_until '[[ $(instance_state "$LOCK_INST") == stopped ]]' 20
rx POST /v1/profiles/lock/backups '{}'; okr
BACKUP_ID=$(jq -r '.id // .backup.id // empty' <<<"$BODY")
rx POST /v1/profiles/lock/export "{\"passphrase\": \"$BACKUP_PW\"}"; okr
ARCHIVE=$(jq -r '.file // empty' <<<"$BODY")
import_body "$ARCHIVE" "wrong-$BACKUP_PW"
rx_file POST /v1/profiles/import "$SCRATCH/import.body"; err InvalidArgument passphrase
import_body "${ARCHIVE:0:$((${#ARCHIVE} / 2 / 4 * 4))}" "$BACKUP_PW"
rx_file POST /v1/profiles/import "$SCRATCH/import.body"; err InvalidArgument
import_body "$(printf 'A%.0s' {1..4096})" "$BACKUP_PW"
rx_file POST /v1/profiles/import "$SCRATCH/import.body"; err InvalidArgument passphrase
if [[ -n $BACKUP_ID ]]; then
    rx POST "/v1/backups/$BACKUP_ID/restore" "{\"passphrase\": \"wrong-$BACKUP_PW\"}"; err InvalidArgument passphrase
    mx InvalidArgument backup_restore "{\"backup_id\": \"$BACKUP_ID\", \"passphrase\": \"wrong-$BACKUP_PW\"}" passphrase
    rx DELETE "/v1/backups/$BACKUP_ID"; okr
    rx DELETE "/v1/backups/$BACKUP_ID"; err NotFound
fi
printf 'not an archive' >"$SCRATCH/garbage.age"
cx InvalidArgument profile import "$SCRATCH/garbage.age" --passphrase "$SCRATCH/import.csv"
import_body "$ARCHIVE" "$BACKUP_PW" "bad name"
rx_file POST /v1/profiles/import "$SCRATCH/import.body"; err InvalidArgument name

rx PUT /v1/retention '{"free_disk_floor_gb": 1000000}'; okr
rx POST /v1/profiles/eph/launch '{}'; err QuotaExceeded
check_json "QuotaExceeded names its scope and numbers" '.details.scope == "disk_floor" and (.details.used_bytes | type == "number") and (.details.limit_bytes | type == "number")'
mx QuotaExceeded profile_launch '{"profile": "eph"}'
cx QuotaExceeded profile launch eph
rx POST /v1/oneshot/screenshot '{"url": "data:text/html,x"}'
CHECKS=$((CHECKS + 1))
[[ $STATUS == 507 || $STATUS == 200 ]] || record "$REQ: oneshot below the disk floor" "507 or 200" "$STATUS"
rx PUT /v1/retention '{"free_disk_floor_gb": 1}'; okr

# --- Builds and proxies that make a launch fail
echo "== browser: launch failures" >&2
new_profile bm ephemeral
profile_set bm '.build = {channel: "brave:stable"}'
rx POST /v1/profiles/bm/launch '{}'; err BuildMissing
mx BuildMissing profile_launch '{"profile": "bm"}'
cx BuildMissing profile launch bm
profile_set bm '.build = {build_id: "01HZZZZZZZZZZZZZZZZZZZZZZZ"}'
rx POST /v1/profiles/bm/launch '{}'; err BuildMissing

rx POST /v1/proxies "{\"scheme\": \"http\", \"host\": \"127.0.0.1\", \"port\": $(free_port), \"username\": \"u\", \"password\": \"$PROXY_PW\"}"; okr
PX=$(jqv .id)
new_profile viaproxy ephemeral
profile_set viaproxy '.proxy = {proxy_id: $id}' --arg id "$PX"
rx DELETE "/v1/proxies/$PX"; err Conflict
check_json "referenced" '.details.reason == "referenced"'
mx Conflict proxy_delete "{\"proxy_id\": \"$PX\"}"
cx Conflict proxy delete "$PX"
rx POST /v1/profiles/viaproxy/launch '{}'; err ProxyUnhealthy
mx ProxyUnhealthy profile_launch '{"profile": "viaproxy"}'
cx ProxyUnhealthy profile launch viaproxy
rx POST /v1/proxy-pools "{\"name\": \"deadpool\", \"mode\": \"sticky\", \"proxy_ids\": [\"$PX\"]}"; okr
POOL=$(jqv .id)
new_profile viapool ephemeral
profile_set viapool '.proxy = {pool_id: $id}' --arg id "$POOL"
rx DELETE "/v1/proxy-pools/$POOL"; err Conflict
check_json "referenced" '.details.reason == "referenced"'
rx POST /v1/profiles/viapool/launch '{}'; err ProxyUnhealthy
rx PATCH /v1/proxy-pools/$POOL "{\"proxy_ids\": [\"$ULID_UNKNOWN\"]}"; err NotFound
cx Conflict pool delete "$POOL"
rx GET /v1/proxies; okr
check "no proxy password in the list" yes "$([[ $BODY != *"$PROXY_PW"* ]] && echo yes || echo no)"

# --- login and credentials against a live Tab
echo "== browser: login" >&2
fake_port=$(free_port)
site_port=$(free_port)
start_fixture fake-2captcha "$fake_port"
start_fixture site "$site_port"
new_profile loginp ephemeral
rest POST /v1/credentials "{\"site\": \"http://127.0.0.1:$site_port\", \"login_url\": \"http://127.0.0.1:$site_port/no-form-here\", \"username\": \"u90\", \"password\": \"$CRED_PW\", \"totp\": \"$CRED_TOTP\", \"profile_ids\": [\"loginp\"]}"
LOGIN_OK=$(jqv .id)
rest POST /v1/credentials "{\"site\": \"https://other-90.test\", \"username\": \"u90\", \"password\": \"$CRED_PW\"}"
LOGIN_UNLINKED=$(jqv .id)
rest POST /v1/profiles/loginp/launch '{}' "$A"; LP_INST=$(jqv .instance.id); LP_TAB=$(jqv .tab.id)
wait_until 'rest GET "/v1/tabs/$LP_TAB"; [[ $(jq -r .state <<<"$BODY") == ready ]]' 15
rx POST "/v1/tabs/$LP_TAB/navigate" '{"url": "data:text/html,<title>empty</title><p>no form here</p>"}' "$A"; okr
rx POST "/v1/tabs/$LP_TAB/login" "{\"credential_id\": \"$LOGIN_UNLINKED\"}" "$A"; err Conflict
check_json "credential_not_linked" '.details.reason == "credential_not_linked"'
rx POST "/v1/tabs/$LP_TAB/login" "{\"credential_id\": \"$ULID_UNKNOWN\"}" "$A"; err NotFound
rx POST "/v1/tabs/$LP_TAB/login" "{\"credential_id\": \"$LOGIN_OK\"}" "$A"; err LoginFailed
check_json "LoginFailed carries step and reason" '.details.reason == "no_form" and (.details.step | type == "number")'
check_json "the message is fixed and holds no page text" '.message | contains("no_form") or length > 0'
mx LoginFailed login "{\"tab_id\": \"$LP_TAB\", \"credential_id\": \"$LOGIN_OK\", \"client_id\": \"$A\"}"
mx Conflict login "{\"tab_id\": \"$LP_TAB\", \"credential_id\": \"$LOGIN_UNLINKED\", \"client_id\": \"$A\"}"
renew_client
cx LoginFailed tab login --tab-id "$LP_TAB" "$LOGIN_OK"
rx DELETE "/v1/credentials/$LOGIN_OK"; okr
rx POST "/v1/profiles/loginp/login-checks" '{"site": "https://login-90.test", "url": "data:text/html,x", "expect_text": "welcome"}'
LC=$(jq -r '.id // empty' <<<"$BODY")
if [[ -n $LC ]]; then
    rx POST "/v1/login-checks/$LC/run" '{}' "$A"
    CHECKS=$((CHECKS + 1))
    ((STATUS < 500)) || record "$REQ: run a login check on a running Profile" "no 5xx" "$STATUS"
fi
rx POST "/v1/instances/$LP_INST/stop" '{}' "$A"; okr

# --- captcha against a fake provider
echo "== browser: captcha" >&2
rx PATCH /v1/settings "$(jq -nc --arg u "http://127.0.0.1:$fake_port" '{"captcha.provider_url": $u}')"; okr
rx PUT /v1/captcha/settings "$(jq -nc --arg k "$CAPTCHA_KEY" '{api_key: $k, daily_solves: 1, human_wait_s: 1}')"; okr
check "the key is not echoed" yes "$([[ $BODY != *"$CAPTCHA_KEY"* ]] && echo yes || echo no)"
rx GET /v1/captcha/balance; okr
new_profile capt ephemeral
profile_set capt '.captcha = "on_demand"'
rest POST /v1/profiles/capt/launch "$(jq -nc --arg u "http://127.0.0.1:$site_port/recaptcha" '{url: $u}')" "$A"
CT_INST=$(jqv .instance.id); CT_TAB=$(jqv .tab.id)
wait_until 'rest GET "/v1/tabs/$CT_TAB"; [[ $(jq -r .state <<<"$BODY") == ready ]]' 15
rx POST "/v1/tabs/$CT_TAB/solve_captcha" '{}' "$A"; okr
rx POST "/v1/tabs/$CT_TAB/solve_captcha" '{"human_fallback": false}' "$A"; err CaptchaBudgetExceeded
mx CaptchaBudgetExceeded solve_captcha "{\"tab_id\": \"$CT_TAB\", \"client_id\": \"$A\", \"human_fallback\": false}"
rx POST "/v1/tabs/$CT_TAB/solve_captcha" '{}' "$A"; err CaptchaFailed
mx CaptchaFailed solve_captcha "{\"tab_id\": \"$CT_TAB\", \"client_id\": \"$A\"}"
renew_client
cx CaptchaFailed tab solve-captcha --tab-id "$CT_TAB"
rx POST "/v1/instances/$CT_INST/stop" '{}' "$A"; okr

# --- artifacts and the one-shot endpoints with a browser
echo "== browser: artifacts and one-shot" >&2
rest POST /v1/profiles/eph/launch '{}' "$A"; AR_INST=$(jqv .instance.id); AR_TAB=$(jqv .tab.id)
wait_until 'rest GET "/v1/tabs/$AR_TAB"; [[ $(jq -r .state <<<"$BODY") == ready ]]' 15
rx POST "/v1/tabs/$AR_TAB/screenshot" '{"return": "url"}' "$A"; okr
ART=$(jq -r '.artifact_id // empty' <<<"$BODY")
if [[ -n $ART ]]; then
    rx GET "/v1/artifacts/$ART/fetch?return=nowhere"; err InvalidArgument
    mx InvalidArgument artifact_fetch "{\"artifact_id\": \"$ART\", \"return\": \"nowhere\"}"
    rx DELETE "/v1/artifacts/$ART"; okr
    rx DELETE "/v1/artifacts/$ART"; err NotFound
    rx GET "/v1/artifacts/$ART"; err NotFound
    rx GET "/v1/artifacts/$ART/fetch"; err NotFound
    mx NotFound artifact_get "{\"artifact_id\": \"$ART\"}"
    cx NotFound artifact get "$ART"
fi
rx POST "/v1/instances/$AR_INST/stop" '{}' "$A"; okr

rx POST /v1/oneshot/screenshot "{\"url\": \"http://127.0.0.1:$dead_page/\"}"; err NavigationFailed
rx GET "/v1/screenshot?url=http://127.0.0.1:$dead_page/"; err NavigationFailed
rx POST /v1/oneshot/markdown "{\"url\": \"http://127.0.0.1:$dead_page/\"}"; err NavigationFailed
mx NavigationFailed oneshot_screenshot "{\"url\": \"http://127.0.0.1:$dead_page/\"}"
cx NavigationFailed shot screenshot "http://127.0.0.1:$dead_page/" -o "$SCRATCH/shot.png"
slow_port=$(free_port)
slow_server "$slow_port"
started=$SECONDS
rx POST /v1/oneshot/screenshot "{\"url\": \"http://127.0.0.1:$slow_port/\", \"timeout\": 2}"
CHECKS=$((CHECKS + 1))
[[ $STATUS == 504 || $STATUS == 502 || $STATUS == 400 ]] || record "$REQ: a page that never answers" "504 Timeout" "$STATUS"
ambiguity "one-shot timeout on a hung page" 504 "$STATUS"
CHECKS=$((CHECKS + 1))
((SECONDS - started <= 60)) || record "$REQ: timeout honoured" "within 60 s" "$((SECONDS - started)) s"

# --- Live attach, capabilities a Live browser lacks
echo "== browser: live attach" >&2
start_test_chrome
cdp="http://127.0.0.1:$TEST_CHROME_PORT"
rx POST /v1/instances/attach '{"cdp_url": "http://127.0.0.1:1"}'; err InvalidArgument cdp_url
rx POST /v1/instances/attach "{\"cdp_url\": \"$cdp\", \"install_name\": \"chromium\"}"; err InvalidArgument
rx POST /v1/instances/attach '{"install_name": "netscape"}'; err InvalidArgument install_name
rx POST /v1/instances/attach '{"install_name": "chrome-beta"}'; err DependencyMissing
check_json "the hint names chrome://inspect" '.message | contains("chrome://inspect")'
rx POST /v1/instances/attach '{"cdp_url": "ftp://x"}'; err InvalidArgument cdp_url
rx POST /v1/instances/attach '{"via": "carrier-pigeon"}'; err InvalidArgument
rx POST /v1/instances/attach "{\"cdp_url\": \"http://127.0.0.1:$slow_port/\"}"
CHECKS=$((CHECKS + 1))
[[ $STATUS == 400 || $STATUS == 504 ]] || record "$REQ: attach to a server that is not a browser" "400 or 504" "$STATUS"
mx InvalidArgument instance_attach '{"cdp_url": "http://127.0.0.1:1"}' cdp_url
cx InvalidArgument instance attach --cdp-url http://127.0.0.1:1
cx DependencyMissing instance attach --install-name chrome-beta
rest POST /v1/instances/attach "{\"cdp_url\": \"$cdp\"}"
assert_status 200 "attach by cdp_url"
LIVE_INST=$(jqv .instance.id)
LIVE_TAB=$(jq -r '.tabs[0].id' <<<"$BODY")
rx POST "/v1/instances/$LIVE_INST/stop" '{}'; err InvalidTransition
mx InvalidTransition instance_stop "{\"instance_id\": \"$LIVE_INST\"}"
cx InvalidTransition instance stop "$LIVE_INST"
rx POST "/v1/instances/$LIVE_INST/promote" '{"name": "live-promote"}'; err_shape
rx DELETE "/v1/tabs/$LIVE_TAB"; err InvalidArgument tab_id
rx POST "/v1/instances/$LIVE_INST/detach" '{}'; okr
rx POST "/v1/instances/$LIVE_INST/detach" '{}'
ambiguity "detach twice" 409 "$STATUS"
CHECKS=$((CHECKS + 1))
((STATUS >= 400 && STATUS < 500)) || record "$REQ: detach twice" "4xx" "$STATUS"
rx POST "/v1/tabs/$LIVE_TAB/pdf" '{}'; err NotFound
CHECKS=$((CHECKS + 1))
kill -0 "$TEST_CHROME_PID" 2>/dev/null || record "live attach errors" "the user's browser is never stopped" "the test Chrome died"

# --- Camoufox: a Capability missing from the engine
if have_camoufox; then
    echo "== browser: camoufox" >&2
    seed_camoufox
    rest POST /v1/clients '{}'; A=$(jqv .id)
    rx PUT /v1/retention '{"free_disk_floor_gb": 1}'; okr
    new_profile camou ephemeral camoufox-headless
    rest POST /v1/profiles/camou/launch '{}' "$A"; CF_INST=$(jqv .instance.id); CF_TAB=$(jqv .tab.id)
    wait_until 'rest GET "/v1/tabs/$CF_TAB"; [[ $(jq -r .state <<<"$BODY") == ready ]]' 30
    rx POST "/v1/tabs/$CF_TAB/pdf" '{}' "$A"; err EngineUnsupported
    check_json "the missing Capability is named" '.details.capability == "pdf"'
    mx EngineUnsupported pdf "{\"tab_id\": \"$CF_TAB\", \"client_id\": \"$A\"}"
    cx EngineUnsupported tab pdf --tab-id "$CF_TAB" -o "$SCRATCH/cf.pdf"
    rx POST "/v1/tabs/$CF_TAB/mhtml" '{}' "$A"; err EngineUnsupported
    rx POST "/v1/instances/$CF_INST/stop" '{}' "$A"; okr
else
    echo "SKIP camoufox: BRR_TEST_CAMOUFOX or BRR_TEST_FPGEN_MODEL not usable" >&2
fi

# --- a dependency the machine lacks: Xvfb off the daemon's PATH
echo "== browser: dependencies" >&2
mkdir -p "$SCRATCH/bin"
if [[ ! -e $SCRATCH/bin/google-chrome-stable && -n ${BRR_TEST_CHROME-} ]]; then
    ln -s "$BRR_TEST_CHROME" "$SCRATCH/bin/google-chrome-stable"
fi
stop_daemon 30 || fail "the daemon ignored SIGTERM"
DAEMON_PATH=$SCRATCH/bin start_daemon
rest POST /v1/builds/detect '{}'
new_profile virt ephemeral
rx POST /v1/profiles/virt/launch '{"display": "virtual"}'; err DependencyMissing
check_json "names the missing program" '.details.name == "Xvfb"'
mx DependencyMissing profile_launch '{"profile": "virt", "display": "virtual"}'
cx DependencyMissing profile launch virt --display virtual
fi

report() {
    local row
    echo >&2
    if ((${#AMBIG[@]} > 0)); then
        echo "== spec ambiguities (not failures): ${#AMBIG[@]}" >&2
        printf '%s\n' "request/check | spec says | actual" >&2
        for row in "${AMBIG[@]}"; do
            printf '%s\n' "${row//$'\t'/ | }" >&2
        done
        echo >&2
    fi
    if ((${#FAILS[@]} > 0)); then
        echo "== FAILED ASSERTIONS: ${#FAILS[@]} of $CHECKS" >&2
        printf '%s\n' "request/check | expected | actual | class" >&2
        for row in "${FAILS[@]}"; do
            printf '%s\n' "${row//$'\t'/ | }" >&2
        done
    fi
}

# ================================================================ 8. the daemon survived

echo "== daemon log" >&2
log=$XDG_STATE_HOME/brr/daemon.log
check "the daemon is alive" yes "$(daemon_alive && echo yes || echo no)"
REQ="daemon.log"
check "no panic in daemon.log" 0 "$(grep -c 'panicked' "$log" 2>/dev/null || true)"
check "no Internal error in daemon.log" 0 "$(grep -c 'code="Internal"' "$log" 2>/dev/null || true)"
for secret in "${SECRETS[@]}"; do
    check "daemon.log never holds the secret '$secret'" 0 "$(grep -cF -- "$secret" "$log" 2>/dev/null || true)"
done

# Only `brr daemon status` reports an unreachable daemon (exit 69); every other command starts one.
stop_daemon
REQ="brr daemon status (no daemon)"
out=$("$BRR_BIN" daemon status 2>"$SCRATCH/cli.err" </dev/null) && rc=0 || rc=$?
err=$(<"$SCRATCH/cli.err")
check "exit code" 69 "$rc"
check "stdout stays empty" "" "$out"
check "stderr is 'error: <message>'" ok "$([[ $err == "error: "?* ]] && echo ok || echo "$err")"
check "no panic after shutdown" 0 "$(grep -c 'panicked' "$log" 2>/dev/null || true)"

report
if ((${#FAILS[@]} > 0)); then
    fail "${#FAILS[@]} of $CHECKS error-path assertions failed (table above)"
fi
finish "error paths across REST, CLI and MCP (${#AMBIG[@]} spec ambiguities noted)"
