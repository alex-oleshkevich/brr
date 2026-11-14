#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail

sse() { # FILE QUERY [LAST_EVENT_ID]: streams in the background until killed by the EXIT trap
    curl -sN --unix-socket "$SOCK" -H "Last-Event-ID: ${3:-0}" "http://localhost/v1/events/stream$2" >"$1" &
}

new_profile one persistent
new_profile two persistent

# Replay from the start, then live events on the same connection.
sse "$SCRATCH/all.txt" ""
wait_until 'grep -q "^event: profile.created" "$SCRATCH/all.txt"' 10
replayed=$(grep -c '^event: profile.created' "$SCRATCH/all.txt")
assert_eq "$replayed" 2 "both earlier profile.created events replayed"
new_profile three persistent
wait_until '[[ $(grep -c "^event: profile.created" "$SCRATCH/all.txt") == 3 ]]' 10

ids=$(sed -n 's/^id: //p' "$SCRATCH/all.txt")
CHECKS=$((CHECKS + 1))
[[ $(wc -l <<<"$ids") -ge 3 ]] || fail "expected at least 3 events, got: $ids"
CHECKS=$((CHECKS + 1))
[[ $(sort -n -u <<<"$ids") == "$ids" ]] || fail "id: lines must strictly increase: $(echo $ids)"
data=$(sed -n 's/^data: //p' "$SCRATCH/all.txt" | head -1)
CHECKS=$((CHECKS + 1))
jq -e '.seq and .ts and .kind and .subject.type' >/dev/null <<<"$data" || fail "SSE data is an Event: $data"

# Last-Event-ID skips what the client already has.
last=$(tail -1 <<<"$ids")
sse "$SCRATCH/tail.txt" "" "$((last - 1))"
wait_until "grep -q '^id: $last\$' \"\$SCRATCH/tail.txt\"" 10
assert_eq "$(grep -c '^id: ' "$SCRATCH/tail.txt")" 1 "only events after Last-Event-ID"

# ?kinds= filters the stream.
sse "$SCRATCH/kinds.txt" "?kinds=profile.created"
wait_until '[[ $(grep -c "^event: " "$SCRATCH/kinds.txt") == 3 ]]' 10
assert_eq "$(grep '^event: ' "$SCRATCH/kinds.txt" | sort -u)" "event: profile.created" "only the requested kind"

# events.list paging: newest first, limit and cursor.
rest GET '/v1/events?limit=1'
assert_status 200 events.list
assert_json '(.events | length == 1) and (.next_cursor | type == "string")'
newest=$(jqv '.events[0].seq')
cursor=$(jqv '.next_cursor')
rest GET '/v1/events?kinds=profile.created&limit=100'
assert_json '.events | length == 3' "kinds filter on events.list"
rest GET '/v1/events?order=asc&limit=2'
assert_json '.events[0].seq < .events[1].seq' "order=asc"

rest GET "/v1/events?instance_id=$(ulid)"
assert_json '.events == []' "instance_id filter excludes other instances"
if have_chrome; then
    detect_builds
    new_profile eph ephemeral
    launch eph
    instance=$(jqv '.instance.id')
    stop_instance "$instance"
    rest GET "/v1/events?instance_id=$instance&limit=100"
    assert_json '.events | length >= 2' "instance events"
    assert_json "[.events[] | .data.instance_id == \"$instance\" or .subject.id == \"$instance\"] | all" "only that instance's events"
fi

# events.watch: empty at the deadline, early on a match.
rest GET '/v1/events?limit=1'
since=$(jqv '.events[0].seq')
start=$SECONDS
rest GET "/v1/events/watch?since=$since&timeout_ms=500"
assert_status 200 events.watch
assert_json '.events == []' "watch times out empty"
CHECKS=$((CHECKS + 1))
((SECONDS - start < 5)) || fail "events.watch took $((SECONDS - start))s for timeout_ms=500"

curl -s --max-time 20 --unix-socket "$SOCK" "http://localhost/v1/events/watch?since=$since&timeout_ms=15000&kinds=profile.created" >"$SCRATCH/watch.json" &
watcher=$!
sleep 0.5
new_profile four persistent
wait "$watcher"
BODY=$(<"$SCRATCH/watch.json")
STATUS=200
assert_json '.events | length == 1 and .[0].kind == "profile.created"' "watch returns on the first match"

# Last: a numeric cursor must be accepted as the string it was issued as.
rest GET "/v1/events?limit=1&cursor=$cursor"
assert_status 200 "events.list with the issued cursor"
assert_json ".events | length == 1 and .[0].seq < $newest" "cursor moves to older events"

finish "SSE replay, Last-Event-ID, kinds, paging, instance filter, watch"
