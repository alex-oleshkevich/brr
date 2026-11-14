#!/usr/bin/env bash
# §7.4: static limits, queue/reject/evict admission and the A2a fallback.
# env: BRR_LIMITS__MAX_TABS_TOTAL=50
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
new_profile ea ephemeral

rest PUT /v1/limits '{"max_instances": 1, "admission": {"queue_timeout_s": 3}}'
assert_status 200 "limits.set"
assert_json '.max_instances == 1 and .admission.queue_timeout_s == 3'

# 1. The first launch fits.
launch ea
a=$(jqv '.instance.id')

# 2. reject answers at once with Retry-After.
rest POST /v1/profiles/ea/launch '{"admission": "reject"}'
assert_status 429 "reject at the cap"
assert_json '.code == "CapExceeded"'
CHECKS=$((CHECKS + 1))
[[ -n $(header_of Retry-After) ]] || fail "429 without Retry-After"

# 3. queue (the default) waits queue_timeout_s, then fails; the row records why.
start=$SECONDS
rest POST /v1/profiles/ea/launch '{}'
waited=$((SECONDS - start))
assert_status 429 "queue timeout"
assert_json '.code == "CapExceeded"'
CHECKS=$((CHECKS + 1))
((waited >= 3 && waited <= 10)) || fail "queued launch answered after ${waited}s, expected 3..10"
rest GET '/v1/instances?state=stopped'
assert_json '[.instances[] | select(.end_reason == "queue_timeout")] | length == 1' "queue_timeout row"

# 4. A queued launch is listed with its position and can be cancelled.
queued_entry() {
    rest GET /v1/admission/queue
    jq -e '.queue | length == 1' >/dev/null <<<"$BODY"
}
curl -s -o "$SCRATCH/c.json" -w '%{http_code}' --max-time 30 --unix-socket "$SOCK" -X POST \
    -H 'content-type: application/json' -d '{}' http://localhost/v1/profiles/ea/launch >"$SCRATCH/c.status" &
c_curl=$!
wait_until queued_entry 3
assert_json '.queue[0].queue_pos == 1' "queue_pos"
c=$(jqv '.queue[0].instance_id')
stop_instance "$c"
assert_json '.state == "stopped" and .end_reason == "cancel"' "cancelled while queued"
wait "$c_curl" || true
CHECKS=$((CHECKS + 1))
[[ $(<"$SCRATCH/c.status") != 200 ]] || fail "the cancelled launch answered 200"

# 5. Tab ceiling.
rest PUT /v1/limits '{"max_tabs_per_instance": 1}'
assert_status 200 "limits.set tabs"
rest POST "/v1/instances/$a/tabs" '{}'
assert_status 429 "second tab over max_tabs_per_instance"
assert_json '.code == "CapExceeded"'
CHECKS=$((CHECKS + 1))
[[ -n $(header_of Retry-After) ]] || fail "tab cap 429 without Retry-After"

# 6. Invalid values and keys pinned by the environment.
rest PUT /v1/limits '{"max_instances": 0}'
assert_status 400 "max_instances 0"
assert_json '.code == "InvalidArgument" and .details.field == "max_instances"'
rest PUT /v1/limits '{"max_tabs_total": 60}'
assert_status 400 "env-pinned limits.max_tabs_total"
assert_json '.code == "InvalidArgument"'
assert_json '.message | contains("BRR_LIMITS__MAX_TABS_TOTAL")'

# 7. evict: a pinned Profile's idle Instance is never a candidate (A2a: falls back to queue);
# once unpinned, the idle Instance is evicted.
rest PUT /v1/limits '{"max_tabs_per_instance": 20}'
set_settings '{"idle.instance_min": 1, "idle.grace_s": 600}'
rest PATCH /v1/profiles/ea '{"pinned": true}'
assert_status 200 "pin profile"
wait_until '[[ $(instance_state "$a") == idle ]]' 100

rest POST /v1/profiles/ea/launch '{"admission": "evict"}'
assert_status 429 "evict with only a pinned candidate falls back to queue"
assert_json '.code == "CapExceeded"'
assert_eq "$(instance_state "$a")" idle "the pinned Instance stays idle"

rest PATCH /v1/profiles/ea '{"pinned": false}'
assert_status 200 "unpin profile"
wait_until '[[ $(instance_state "$a") == reaping ]]' 100

launch_evict() {
    rest POST /v1/profiles/ea/launch '{"admission": "evict"}'
}
launch_evict
assert_status 200 "evict the unpinned idle Instance"
d=$(jqv '.instance.id')
rest GET "/v1/instances/$a"
assert_json '.state == "stopped" and .end_reason == "evicted"' "A evicted"
CHECKS=$((CHECKS + 1))
state_trail "$a" | grep -qx 'stopping:evict' || state_trail "$a" | grep -qx 'stopping:grace_over' || fail "A's trail lacks stopping:evict|grace_over: $(state_trail "$a" | tr '\n' ' ')"
rest GET '/v1/events?kinds=admission.decided&limit=1000'
assert_json '[.events[] | select(.data.outcome == "evicted")] | length >= 1' "admission.decided evicted"
stop_instance "$d"

finish "limits, reject/queue/evict admission, Retry-After, env-pinned limits"
