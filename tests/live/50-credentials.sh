#!/usr/bin/env bash
# M6: the credential vault over REST and CLI: secrets never come back, validation, CSV import.
source "$(dirname "$0")/lib.sh"
set -euo pipefail

PASSWORD=S3cret-Live-9f2
TOTP=JBSWY3DPEHPK3PXP
NOTES=note-secret-77

# no_secrets TEXT LABEL: none of the three secret strings is in TEXT.
no_secrets() {
    local secret
    for secret in "$PASSWORD" "$TOTP" "$NOTES"; do
        assert_eq "$(grep -cF -- "$secret" <<<"$1" || true)" 0 "$2 holds $secret"
    done
}

new_profile cred-profile ephemeral
rest POST /v1/credentials "$(jq -nc --arg p "$PASSWORD" --arg t "$TOTP" --arg n "$NOTES" \
    '{site: "https://example.test/login", username: "alice", password: $p, totp: $t, notes: $n, profile_ids: ["cred-profile"]}')"
assert_status 200 credential.create
assert_json '.totp_set == true and .site == "https://example.test"' "site is normalized to its origin"
id=$(jqv '.id')
no_secrets "$BODY" "create response"

rest GET /v1/credentials
assert_status 200 credential.list
CHECKS=$((CHECKS + 1))
jq -e --arg id "$id" '.credentials[] | select(.id == $id) | .totp_set == true and .username == "alice"' >/dev/null <<<"$BODY" ||
    fail "list shows totp_set:true"
no_secrets "$BODY" "list response"
no_secrets "$(brr credential list --json)" "CLI list"

# An update without a password keeps the stored one.
rest PATCH "/v1/credentials/$id" '{"username": "alice2"}'
assert_status 200 "update without password"
assert_json '.username == "alice2" and .totp_set == true' "other fields kept"
no_secrets "$BODY" "update response"

rest PATCH "/v1/credentials/$id" '{"totp": null}'
assert_status 200 "totp null"
assert_json '.totp_set == false' "totp:null removes the secret"

rest PATCH "/v1/credentials/$id" '{"totp": "not base32!"}'
assert_status 400 "bad totp"
assert_json '.code == "InvalidArgument" and (.details.field | length > 0)'

rest DELETE "/v1/credentials/$id"
assert_status 200 credential.delete
assert_json '.deleted == true'
rest PATCH "/v1/credentials/$id" '{"username": "gone"}'
assert_status 404 "update after delete"
assert_json '.code == "NotFound"'

rest POST /v1/credentials '{"site": "ftp://x", "username": "u", "password": "p"}'
assert_status 400 "ftp site"
assert_json '.code == "InvalidArgument" and .details.field == "site"'

# --- CSV import: one new login, one already in the vault, one with a broken URL.
rest POST /v1/credentials '{"site": "https://dup.example", "username": "alice", "password": "old-pw"}'
assert_status 200 "seed the duplicate"
printf '%s\n' 'name,url,username,password,note' \
    'dup,https://dup.example/login,alice,pw-1,' \
    'new,https://new.example/login,bob,pw-2,hello' \
    'broken,not a url,carol,pw-3,' >"$SCRATCH/chrome.csv"
out=$(brr credential import "$SCRATCH/chrome.csv" --format chrome --json)
assert_eq "$(jq -c . <<<"$out")" '{"imported":1,"skipped":2}' "chrome import counts"

rest GET /v1/credentials
assert_json '[.credentials[] | select(.username == "bob")] | length == 1' "the new login is in the vault"
assert_json '[.credentials[] | select(.username == "carol")] | length == 0' "the broken row was not imported"

printf '%s\n' 'url,username,password,httpRealm,formActionOrigin,guid,timeCreated,timeLastUsed,timePasswordChanged' \
    'https://ff.example,dave,pw-4,,https://ff.example,{1},1,1,1' >"$SCRATCH/firefox.csv"
rc=0
brr credential import "$SCRATCH/firefox.csv" --format chrome --json >"$SCRATCH/ff.out" 2>"$SCRATCH/ff.err" || rc=$?
CHECKS=$((CHECKS + 1))
((rc != 0)) || fail "a Firefox file read as Chrome must fail"
rest POST /v1/credentials/import "$(jq -nc --rawfile f "$SCRATCH/firefox.csv" '{format: "chrome", file: $f}')"
assert_status 400 "Firefox file as chrome"
assert_json '.code == "InvalidArgument" and .details.field == "file"'

no_secrets "$(cat "$XDG_STATE_HOME/brr/daemon.log")" "daemon.log"

finish "credential CRUD, secrets never returned, validation, CSV import"
