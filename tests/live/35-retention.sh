#!/usr/bin/env bash
# Retention: the dry run lists exactly what the real sweep then deletes.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
http_server "$M4_FIXTURES"
base=http://127.0.0.1:$HTTP_PORT

ids=()
for page in form.html article.html; do
    rest GET "/v1/screenshot?url=$base/$page&return=url"
    assert_status 200 "capture $page"
    ids+=("$(jqv '.artifact_id')")
done

# An ephemeral data dir no Instance owns, old enough to be an orphan.
orphan=$(ulid)
mkdir -p "$XDG_DATA_HOME/brr/ephemeral/$orphan"
echo junk >"$XDG_DATA_HOME/brr/ephemeral/$orphan/file"
touch -d '3 days ago' "$XDG_DATA_HOME/brr/ephemeral/$orphan"

rest PUT /v1/retention '{"artifacts_max_age_days": 0}'
assert_status 200 retention.set

pairs() { # REPORT: sorted "rule id" pairs
    jq -r '.groups[] | .rule as $r | .items[] | "\($r) \(.id)"' <<<"$1" | sort
}

rest POST /v1/retention/sweep '{"dry_run": true}'
assert_status 200 "dry run"
dry=$BODY
dry_pairs=$(pairs "$dry")
for id in "${ids[@]}"; do
    CHECKS=$((CHECKS + 1))
    grep -q "^artifacts_age $id\$" <<<"$dry_pairs" || fail "the dry run lacks artifact $id: $dry_pairs"
done
CHECKS=$((CHECKS + 1))
grep -q "$orphan" <<<"$dry_pairs" || fail "the dry run lacks the orphan dir $orphan: $dry_pairs"
CHECKS=$((CHECKS + 1))
[[ -d $XDG_DATA_HOME/brr/ephemeral/$orphan ]] || fail "the dry run deleted the orphan dir"
rest GET "/v1/artifacts/${ids[0]}"
assert_status 200 "the dry run deleted nothing"

rest POST /v1/retention/sweep '{"dry_run": false}'
assert_status 200 "real sweep"
real_pairs=$(pairs "$BODY")
assert_eq "$real_pairs" "$dry_pairs" "the real sweep deletes exactly what the dry run listed"
for id in "${ids[@]}"; do
    rest GET "/v1/artifacts/$id"
    assert_status 404 "artifact $id after the sweep"
done
CHECKS=$((CHECKS + 1))
[[ ! -e $XDG_DATA_HOME/brr/ephemeral/$orphan ]] || fail "the orphan dir survives the sweep"

rest POST /v1/retention/sweep '{"dry_run": true}'
assert_status 200 "second dry run"
second=$(pairs "$BODY" | grep -E "^(artifacts_age|ephemeral)" || true)
assert_eq "$second" "" "nothing left to sweep for those rules"

rest GET '/v1/events?kinds=retention.sweep_completed&limit=20'
# A dry run changes nothing, so only the real sweep publishes.
assert_json '[.events[] | select(.data.dry_run == true)] | length == 0' "no sweep_completed event for a dry run"
assert_json '.events | length == 1 and .[0].data.dry_run == false' "exactly the real sweep's event"

finish "dry run == real sweep over ${#ids[@]} artifacts and an orphan dir"
