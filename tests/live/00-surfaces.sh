#!/usr/bin/env bash
# R1: every operation in the registry is reachable through REST, MCP and the CLI.
source "$(dirname "$0")/lib.sh"
set -euo pipefail

spec=$SCRATCH/openapi.json
echo "name,url,username,password" >"$SCRATCH/import.csv"
curl -sS --unix-socket "$SOCK" http://localhost/openapi.json >"$spec"

# Minimal input for each operation: a fresh ULID for every path parameter and a dummy for every
# required body field (from the OpenAPI schema), overridden by the table for the cases that need
# a meaningful value. {s} is the surface, so no two calls collide on names.
input_for() { # OP SURFACE -> JSON
    jq -nc --slurpfile spec "$spec" --arg op "$1" --arg s "$2" --arg u "$(ulid)" --arg csv "$SCRATCH/import.csv" '
    def pick: if type == "object" and has("$ref") then ($spec[0].components.schemas[.["$ref"] | split("/") | last] | pick)
        elif type == "object" and has("anyOf") then (.anyOf | map(select(.type != "null")) | .[0] | pick) else . end;
    def ty: (.type | if type == "array" then map(select(. != "null"))[0] else . end);
    def dummy: pick | if has("enum") then .enum[0]
        elif has("x-brr-file") then $csv
        elif has("oneOf") then (.oneOf[0] | dummy)
        elif ty == "string" then "x"
        elif ty == "integer" or ty == "number" then 1
        elif ty == "boolean" then false
        elif ty == "array" then []
        elif ty == "object" then (. as $o | ($o.required // []) | map({key: ., value: ($o.properties[.] | dummy)}) | from_entries)
        else null end;
    ([$spec[0].paths[][] | select(type == "object" and .operationId == $op)][0]) as $o
    | (([$o.parameters[]? | select(.in == "path") | {key: .name, value: $u}] | from_entries)
       + ($o.requestBody.content["application/json"].schema // {} | pick | if .type == "object" then dummy else {} end))
    * ({
      "build.install": {source: "system"},
      "build.available": {source: "system"},
      "credential.create": {site: ("site-" + $s + ".test"), username: "u", password: "p"},
      "events.watch": {timeout_ms: 100},
      "extension.add": {ref: "not-an-extension"},
      "profile.create": {name: ("p-" + $s), template: "chromium-headless"},
      "template.save_from_profile": {name: ("t-" + $s)},
      "settings.set": {"ui.theme": "dark"},
      "tab.navigate": {url: "about:blank"},
      "template.create": {name: ("t-" + $s), settings: {}}
    }[$op] // {})'
}

# --- REST: a JSON body that is an output or {code,message,details}; never a bare 404/405.
answered_rest() { # OP METHOD PATH INPUT
    local op=$1 method=$2 path=$3 input=$4 key query='' body='{}'
    for key in $(grep -o '{[a-z_]*}' <<<"$path" | tr -d '{}'); do
        path=${path//\{$key\}/$(jq -r --arg k "$key" '.[$k]' <<<"$input")}
        input=$(jq -c --arg k "$key" 'del(.[$k])' <<<"$input")
    done
    if [[ $method == GET ]]; then
        query=$(jq -r 'to_entries | map("\(.key)=\(.value | tostring | @uri)") | join("&")' <<<"$input")
        [[ -z $query ]] || path="$path?$query"
        rest GET "$path"
    elif [[ $method == DELETE ]]; then
        rest DELETE "$path"
    else
        [[ $input == '{}' ]] || body=$input
        rest "$method" "$path" "$body"
    fi
    jq -e 'type == "object"' >/dev/null 2>&1 <<<"$BODY" || fail "REST $op: not a JSON object (HTTP $STATUS)"
    if ((STATUS >= 400)); then
        jq -e 'has("code") and has("message") and has("details")' >/dev/null <<<"$BODY" || fail "REST $op: HTTP $STATUS without an error body"
    fi
    CHECKS=$((CHECKS + 1))
}

# --- MCP: a tool result that is structuredContent or an error text `<code>: <message>` + body JSON.
answered_mcp() { # OP TOOL INPUT
    mcp_tool "$2" "$3"
    if [[ $MCP_ERROR == true ]]; then
        [[ $MCP_TEXT =~ ^[A-Za-z]+:\  ]] || fail "MCP $1: error text lacks '<code>: ': $MCP_TEXT"
        jq -e 'has("code") and has("message")' >/dev/null <<<"$BODY" || fail "MCP $1: no error body on line 2: $MCP_TEXT"
    else
        jq -e 'type == "object"' >/dev/null <<<"$BODY" || fail "MCP $1: no structuredContent: $MCP_RAW"
    fi
    CHECKS=$((CHECKS + 1))
}

# --- CLI: path words, then the input as flags or positionals read off `--help`.
answered_cli() { # OP "CLI PATH" INPUT
    local op=$1 cli=$2 input=$3 help usage flag name value kind
    local -a positionals=() argv=()
    declare -A given=()
    read -ra words <<<"$cli"
    help=$("$BRR_BIN" "${words[@]}" --help)
    usage=$(sed -n 's/^Usage: //p' <<<"$help" | head -1)
    if [[ $op == settings.set ]]; then
        argv=("$(jq -r 'keys[0]' <<<"$input")" "$(jq -r '.[]' <<<"$input")")
    else
        read -ra positionals <<<"$(grep -o '<[A-Za-z_]*>' <<<"$usage" | tr -d '<>' | tr '\n' ' ')"
        positionals=("${positionals[@]/PATH/file}")
        while IFS=$'\t' read -r name kind value; do
            flag=--${name//_/-}
            if grep -qE -- "^ +(-[a-z], )?$flag( |\[|\$)" <<<"$help"; then
                case $kind in
                boolean) argv+=("$flag=$value") ;;
                *) argv+=("$flag" "$value") ;;
                esac
            else
                given[$name]=$value
            fi
        done < <(jq -r 'to_entries[] | [.key, (.value | type), (.value | if type == "string" then . else tojson end)] | @tsv' <<<"$input")
        for name in "${positionals[@]}"; do
            [[ -v given[$name] ]] && argv+=("${given[$name]}")
        done
    fi

    local out rc=0 err
    out=$(timeout 60 "$BRR_BIN" --json "${words[@]}" "${argv[@]}" 2>"$SCRATCH/cli.err") || rc=$?
    err=$(<"$SCRATCH/cli.err")
    if ((rc == 0)); then
        jq -e 'type == "object"' >/dev/null 2>&1 <<<"$out" || fail "CLI $op ($cli ${argv[*]}): exit 0 but stdout is not a JSON object: $out"
    else
        ((rc >= 1 && rc <= 22)) || fail "CLI $op ($cli ${argv[*]}): exit $rc (transport or usage failure): $err"
        [[ -z $out && $err == error:* && $err != *Usage:* ]] || fail "CLI $op ($cli ${argv[*]}): exit $rc without a typed error: out='$out' err='$err'"
    fi
    CHECKS=$((CHECKS + 1))
}

total=$(jq '[.paths[][] | select(type == "object" and has("operationId"))] | length' "$spec")
rest_done=0 mcp_done=0 cli_done=0 no_mcp=0 no_cli=0

mcp_http tools/list '{}'
tools=$(jq -r '.result.tools[].name' <<<"$MCP_RAW")

while IFS=$'\t' read -r op method path tool cli; do
    for surface in rest mcp cli; do
        input=$(input_for "$op" "$surface")
        case $surface in
        rest) answered_rest "$op" "$method" "$path" "$input"; rest_done=$((rest_done + 1)) ;;
        mcp)
            # Human-only operations (take over, hand back) are deliberately not MCP tools.
            if [[ $tool == - ]]; then
                no_mcp=$((no_mcp + 1))
                continue
            fi
            grep -qx "$tool" <<<"$tools" || fail "MCP tools/list lacks $tool ($op)"
            answered_mcp "$op" "$tool" "$input"
            mcp_done=$((mcp_done + 1))
            ;;
        cli)
            # Human-only operations (take over, hand back, human done) are the web UI's: no command.
            if [[ $cli == - ]]; then
                no_cli=$((no_cli + 1))
                continue
            fi
            "$BRR_BIN" $cli --help >/dev/null 2>&1 || fail "brr $cli --help failed ($op)"
            answered_cli "$op" "$cli" "$input"
            cli_done=$((cli_done + 1))
            ;;
        esac
    done
done < <(jq -r '.paths | to_entries[] | .key as $p | .value | to_entries[] | select(.value | type == "object" and has("operationId"))
    | [.value.operationId, (.key | ascii_upcase), $p, (.value["x-brr-mcp-tool"] // "-"), (.value["x-brr-cli"] // "-")] | @tsv' "$spec")

assert_eq "$rest_done" "$total" "operations answered over REST"
assert_eq "$((mcp_done + no_mcp))" "$total" "operations answered over MCP or declared without a tool"
assert_eq "$((cli_done + no_cli))" "$total" "operations answered over the CLI or declared without a command"

finish "$total operations answered through REST, $cli_done through the CLI ($no_cli UI-only), $mcp_done through MCP ($no_mcp without an MCP tool)"
