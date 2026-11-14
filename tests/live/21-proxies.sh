#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

PASSWORD=s3cret-pw-live-$RANDOM
USER_NAME=u
detect_builds

trace_port=$(free_port)
upstream_port=$(free_port)
peer_port=$(free_port)
dead_port=$(free_port)
trace_server "$trace_port"
peer_server "$peer_port"
upstream_proxy "$upstream_port" "$USER_NAME" "$PASSWORD"
PEER_URL=http://127.0.0.1:$peer_port/

rest PATCH /v1/settings "$(jq -nc --arg u "http://127.0.0.1:$trace_port/" '{"proxy.check_url": $u}')"
assert_status 200 "proxy.check_url"

proxy_healthy() {
    rest GET /v1/proxies
    jq -e --arg id "$1" '.proxies[] | select(.id == $id) | .healthy == true' >/dev/null <<<"$BODY"
}

# 2. A proxy with a password: the response carries a status, never the secret; health gets the exit ip.
rest POST /v1/proxies "$(jq -nc --arg p "$PASSWORD" --argjson port "$upstream_port" \
    '{scheme: "http", host: "127.0.0.1", port: $port, username: "u", password: $p}')"
assert_status 200 proxy.create
assert_json '.password.set == true and (.password.hint | type == "string")' "password is a status"
live_id=$(jqv '.id')
wait_until "proxy_healthy $live_id" 30
assert_json ".proxies[] | select(.id == \"$live_id\") | .exit_ip == \"203.0.113.7\"" "exit ip through the upstream"

rest POST /v1/proxies "$(jq -nc --argjson port "$dead_port" '{scheme: "http", host: "127.0.0.1", port: $port}')"
assert_status 200 "dead proxy"
dead_id=$(jqv '.id')
rest POST "/v1/proxies/$dead_id/check" '{}'
assert_status 200 proxy.check
assert_json '.healthy == false' "dead proxy is unhealthy"

# 4. §18: a Profile through the proxy shows the peer the upstream's address.
new_profile via-proxy ephemeral
profile_id=$(jqv '.id')
profile_set "$profile_id" '.proxy = {proxy_id: $id}' --arg id "$live_id"
if can_bind_loopback2; then
    launch via-proxy "" "$PEER_URL"
    inst=$(jqv '.instance.id')
    tab=$(jqv '.tab.id')
    wait_title "$tab" "peer:127.0.0.2" 30
    stop_instance "$inst"
    wait_until '[[ $(instance_state "$inst") == stopped ]]' 15
    exit_checked="peer saw 127.0.0.2"
else
    exit_checked="127.0.0.2 step skipped (no full loopback)"
fi

# 3. The password never appears in a response, an event or the daemon log.
rest GET /v1/proxies
CHECKS=$((CHECKS + 1))
[[ $BODY != *"$PASSWORD"* ]] || fail "proxy list leaks the password"
rest GET "/v1/profiles/$profile_id"
[[ $BODY != *"$PASSWORD"* ]] || fail "profile leaks the password"
events=$(curl -sS --unix-socket "$SOCK" -H "Last-Event-ID: 0" --max-time 2 http://localhost/v1/events/stream 2>/dev/null || true)
[[ $events != *"$PASSWORD"* ]] || fail "SSE events leak the password"
rest GET '/v1/events?limit=500'
[[ $BODY != *"$PASSWORD"* ]] || fail "events.list leaks the password"
CHECKS=$((CHECKS + 1))
! grep -qF -- "$PASSWORD" "$XDG_STATE_HOME/brr/daemon.log" || fail "daemon.log leaks the password"

# 5. Import: one new, one duplicate, one invalid line.
rest POST /v1/proxies/import "$(jq -nc --argjson p "$upstream_port" \
    '{scheme: "http", text: "10.9.9.9:8080\n10.9.9.9:8080\nnot a proxy line"}')"
assert_status 200 proxy.import
assert_json '.imported == 1 and .duplicates == 1 and .invalid_lines == [3]' "import counts"

# 6. Pools: sticky [dead, live] reaches the peer through the live one; dead-only fails.
rest POST /v1/proxy-pools "$(jq -nc --arg d "$dead_id" --arg l "$live_id" '{name: "mixed", mode: "sticky", proxy_ids: [$d, $l]}')"
assert_status 200 pool.create
mixed_id=$(jqv '.id')
rest POST /v1/proxy-pools "$(jq -nc --arg d "$dead_id" '{name: "dead-only", mode: "sticky", proxy_ids: [$d]}')"
assert_status 200 "dead pool"
dead_pool_id=$(jqv '.id')

new_profile pooled ephemeral
pooled=$(jqv '.id')
profile_set "$pooled" '.proxy = {pool_id: $id}' --arg id "$mixed_id"
if can_bind_loopback2; then
    for _ in 1 2; do
        launch pooled "" "$PEER_URL"
        inst=$(jqv '.instance.id')
        tab=$(jqv '.tab.id')
        wait_title "$tab" "peer:127.0.0.2" 30
        stop_instance "$inst"
        wait_until '[[ $(instance_state "$inst") == stopped ]]' 15
    done
fi

profile_set "$pooled" '.proxy = {pool_id: $id}' --arg id "$dead_pool_id"
rest POST "/v1/profiles/pooled/launch" '{}'
assert_status 502 "a pool of dead proxies"
assert_json '.code == "ProxyUnhealthy"'

# 7. A fixed dead proxy fails the launch and leaves a failed instance row.
new_profile fixed-dead ephemeral
fixed=$(jqv '.id')
profile_set "$fixed" '.proxy = {proxy_id: $id}' --arg id "$dead_id"
rest POST "/v1/profiles/fixed-dead/launch" '{}'
assert_status 502 "a fixed dead proxy"
assert_json '.code == "ProxyUnhealthy"'

# 8. A proxy in use cannot be deleted.
rest DELETE "/v1/proxies/$live_id"
assert_status 409 "proxy in use"
assert_json '.code == "Conflict" and .details.reason == "referenced"'

# Last, so a daemon bug here does not hide the steps above.
rest GET "/v1/instances?profile=$fixed"
assert_json '[.instances[] | select(.state == "failed")] | length >= 1' "the instance row is failed"

finish "create, health, secrets, import, pools, dead proxies, referenced; $exit_checked"
