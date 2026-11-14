#!/usr/bin/env bash
# M6: content cleanup: ad blocking from a local filter list, cookie banners dismissed (Chrome one-shot, Camoufox Profile).
# env: BRR_RETENTION__FREE_DISK_FLOOR_GB=1
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
site_port=$(free_port)
start_fixture site "$site_port"
base=http://127.0.0.1:$site_port

set_settings "$(jq -nc --arg u "$base/lists/easylist.txt" '{"content.adblock_lists": [$u]}')"

# --- ad blocking: the page asks for /ad.js and /ok.js, the filter list only knows /ad.js.
new_profile ads ephemeral
profile_set ads '.block_ads = true'
launch ads "" "$base/ads"
instance=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
hits() { curl -sS --max-time 5 "$base/hits"; }
wait_until 'jq -e "index(\"ok.js\") != null" >/dev/null <<<"$(hits)"' 30
CHECKS=$((CHECKS + 1))
jq -e 'index("ad.js") == null' >/dev/null <<<"$(hits)" || fail "ad.js reached the server: $(hits)"
tab_call "$tab" evaluate '{"expression": "[window.okLoaded === true, window.adLoaded === true]"}'
assert_status 200 evaluate
assert_json '.result == [true, false]' "the page ran ok.js and never ad.js"
stop_instance "$instance"

# Without block_ads on the Profile the same page loads both.
new_profile plain ephemeral
launch plain "" "$base/ads"
plain_instance=$(jqv '.instance.id')
wait_until 'jq -e "index(\"ad.js\") != null" >/dev/null <<<"$(hits)"' 30
stop_instance "$plain_instance"

# --- cookie banners on a one-shot capture
raw_content() { # EXTRA_QUERY: GET /v1/content as raw HTML in $BODY
    local out=$SCRATCH/content.out
    STATUS=$(curl -sS --max-time 120 --unix-socket "$SOCK" -o "$out" -w '%{http_code}' \
        "http://localhost/v1/content?url=$base/cookies$1")
    BODY=$(<"$out")
}
raw_content ''
assert_status 200 "content without dismiss_cookies"
CHECKS=$((CHECKS + 1))
[[ $BODY == *cookie-notice* ]] || fail "the notice is gone without dismiss_cookies"
raw_content '&dismiss_cookies=true'
assert_status 200 "content with dismiss_cookies"
CHECKS=$((CHECKS + 1))
[[ $BODY != *cookie-notice* ]] || fail "the notice is still there with dismiss_cookies"
CHECKS=$((CHECKS + 1))
[[ $BODY == *content* ]] || fail "the page body is missing: ${BODY:0:300}"

# --- Camoufox: the same banner on a Profile that dismisses cookies (last: seeding restarts the daemon).
camoufox_note=", Camoufox part skipped (no BRR_TEST_CAMOUFOX and BRR_TEST_FPGEN_MODEL)"
if have_camoufox; then
    camoufox_note=" and on Camoufox"
    seed_camoufox
    new_profile fox ephemeral camoufox-headless
    profile_set fox '.dismiss_cookies = true'
    launch fox "" "$base/cookies"
    fox_instance=$(jqv '.instance.id')
    fox_tab=$(jqv '.tab.id')
    consent_set() {
        tab_call "$fox_tab" evaluate '{"expression": "document.cookie"}'
        jq -e '.result | contains("consent=rejected")' >/dev/null <<<"$BODY"
    }
    wait_until consent_set 30
    stop_instance "$fox_instance"
fi

finish "ad.js blocked while ok.js loads, banner dismissed on the Chrome one-shot$camoufox_note"
