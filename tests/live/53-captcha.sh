#!/usr/bin/env bash
# M6: captcha against a fake 2captcha: auto-solve on load, budget, awaiting_human, CaptchaFailed, metrics.
# env: BRR_RETENTION__FREE_DISK_FLOOR_GB=1
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

API_KEY=2cap-LIVE-KEY-7781

detect_builds
fake_port=$(free_port)
site_port=$(free_port)
start_fixture fake-2captcha "$fake_port"
start_fixture site "$site_port"
base=http://127.0.0.1:$site_port

set_settings "$(jq -nc --arg u "http://127.0.0.1:$fake_port" '{"captcha.provider_url": $u}')"
rest GET /v1/captcha/settings
assert_status 200 captcha.settings_get
assert_json '.api_key == null and .provider == "2captcha"' "no key yet"

rest GET /v1/captcha/balance
assert_status 400 "balance without a key"
assert_json '.code == "InvalidArgument" and .details.field == "api_key"'

rest PUT /v1/captcha/settings "$(jq -nc --arg k "$API_KEY" '{api_key: $k, daily_solves: 1, human_wait_s: 5}')"
assert_status 200 captcha.settings_set
assert_json '.api_key.set == true and .daily_solves == 1 and .human_wait_s == 5'
CHECKS=$((CHECKS + 1))
[[ $BODY != *"$API_KEY"* ]] || fail "settings_set returned the key"
rest GET /v1/captcha/settings
assert_json '.api_key.set == true and (.api_key.hint | type == "string")'
CHECKS=$((CHECKS + 1))
[[ $BODY != *"$API_KEY"* ]] || fail "settings_get returned the key"

rest GET /v1/captcha/balance
assert_status 200 captcha.balance
assert_json '.balance_usd_micros == 3500000' "the fake account balance"

new_profile cap ephemeral
profile_set cap '.captcha = "auto"'
rest POST /v1/clients '{}'
client=$(jqv '.id')

page_title() { # TAB: the page's own title, in $BODY's .title
    tab_call "$1" get_metadata '{}' "$client"
    jq -r '.title // ""' <<<"$BODY"
}

# --- 1. Auto policy: loading the page detects, solves through the fake provider and applies the token.
launch cap "$client" "$base/recaptcha"
instance=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
wait_until '[[ $(page_title "$tab") == solved:tok-1 ]]' 60

rest GET /v1/captcha/solves
assert_status 200 captcha.solves
assert_json '.solves | length == 1' "one solve logged"
assert_json '.solves[0].result == "solved" and .solves[0].cost_usd_micros == 2990 and .solves[0].type == "recaptcha_v2"'
assert_json ".solves[0].site == \"$base\""
rest GET "/v1/events?kinds=captcha.detected,captcha.solved&order=asc&limit=10"
assert_json '[.events[].kind] == ["captcha.detected", "captcha.solved"]' "detected, then solved"
assert_json '.events[1].data.cost_usd == 0.00299'

# --- 2. The budget is spent: the second captcha goes to a person, then fails.
profile_set cap '.captcha = "on_demand"'
tab_call "$tab" navigate "$(jq -nc --arg u "$base/recaptcha" '{url: $u}')" "$client"
assert_status 200 navigate

started=$SECONDS
tab_call "$tab" solve_captcha '{}' "$client"
elapsed=$((SECONDS - started))
assert_status 422 "nobody solved it"
assert_json '.code == "CaptchaFailed"'
CHECKS=$((CHECKS + 1))
((elapsed >= 4 && elapsed <= 15)) || fail "CaptchaFailed came after ${elapsed}s, expected 4 to 15"

rest GET "/v1/events?kinds=tab.awaiting_human,captcha.budget_exhausted&order=asc&limit=10"
assert_json '[.events[] | select(.kind == "tab.awaiting_human") | .data.reason] == ["captcha"]' "the Tab waited for a person"
assert_json '[.events[] | select(.kind == "captcha.budget_exhausted")] | length == 1' "budget_exhausted once"

tab_call "$tab" solve_captcha '{"human_fallback": false}' "$client"
assert_status 402 "no fallback"
assert_json '.code == "CaptchaBudgetExceeded"'

# --- metrics and the fake's view of the requests
metrics=$(curl -sS --max-time 10 "http://$BIND/metrics")
for line in 'brr_captcha_solves_total{result="solved"} 1' 'brr_captcha_solves_total{result="failed"} 0' 'brr_captcha_spend_usd_total 0.002990'; do
    CHECKS=$((CHECKS + 1))
    grep -qxF -- "$line" <<<"$metrics" || fail "/metrics lacks: $line"
done
requests=$(curl -sS "http://127.0.0.1:$fake_port/_requests")
BODY=$requests
assert_json '[.[] | select(.path == "/createTask")] | length == 1' "one paid task"
assert_json '[.[] | select(.path == "/createTask")][0].body.task | .type == "RecaptchaV2TaskProxyless" and .websiteKey == "live-sitekey"'
CHECKS=$((CHECKS + 1))
! grep -qF -- "$API_KEY" "$XDG_STATE_HOME/brr/daemon.log" || fail "daemon.log holds the API key"

stop_instance "$instance"
finish "auto-solve, budget, awaiting_human then CaptchaFailed after ${elapsed}s, 402, metrics"
