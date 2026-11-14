#!/usr/bin/env bash
# P6, D1: export/import across data roots, backups on local storage and rustfs, scheduled backups.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

PASS_TEXT=live-passphrase-$(ulid)
detect_builds
peer_port=$(free_port)
peer_server "$peer_port"

stop_and_wait() {
    stop_instance "$1"
    wait_until "[[ \$(instance_state $1) == stopped ]]" 15
}

# The 60 MiB archive below must not trip the disk floor on a nearly full dev machine; the floor
# itself is covered by the daemon tests.
set_settings '{"retention.free_disk_floor_gb": 0}'

# --- the passphrase is a secret: reported as set, never echoed, never in config.toml
set_settings "$(jq -nc --arg p "$PASS_TEXT" '{"backup.passphrase": $p}')"
CHECKS=$((CHECKS + 1))
[[ $BODY != *"$PASS_TEXT"* ]] || fail "settings.set repeats the passphrase"
rest GET /v1/settings
assert_status 200 settings.get
assert_json '.secrets["backup.passphrase"].set == true' "the passphrase shows as set"
CHECKS=$((CHECKS + 1))
[[ $BODY != *"$PASS_TEXT"* ]] || fail "settings.get repeats the passphrase"
CHECKS=$((CHECKS + 1))
[[ $(grep -rc -- "$PASS_TEXT" "$XDG_CONFIG_HOME" | awk -F: '{s += $2} END {print s + 0}') == 0 ]] ||
    fail "the passphrase is in the config dir"

# --- a Persistent Profile with a cookie set by a real page
new_profile jar persistent
launch jar "" "http://127.0.0.1:$peer_port/set"
inst=$(jqv '.instance.id')
wait_title "$(jqv '.tab.id')" "cookie-set" 30

# A running Profile is not backed up: 409 and a backup.skipped event.
rest POST /v1/profiles/jar/backups '{}'
assert_status 409 "backup of a running Profile"
assert_json '.code == "Conflict" and .details.reason == "profile_running"'
rest GET "/v1/events?kinds=backup.skipped"
assert_json '[.events[] | select(.kind == "backup.skipped")] | length == 1' "a backup.skipped event"
stop_and_wait "$inst"

# --- export / import across two data roots
printf '%s\n' "$PASS_TEXT" >"$SCRATCH/pp.txt"
printf 'not-the-passphrase\n' >"$SCRATCH/wrong.txt"
brr profile export jar -o "$SCRATCH/jar.age" --passphrase "$SCRATCH/pp.txt" >/dev/null
CHECKS=$((CHECKS + 1))
[[ $(head -c 21 "$SCRATCH/jar.age") == "age-encryption.org/v1" ]] || fail "the archive is not an age file"

start_second_daemon
rest2 PATCH /v1/settings '{"retention.free_disk_floor_gb": 0}'
assert_status 200 "second daemon: no disk floor"
rc=0
err=$(brr2 profile import --file "$SCRATCH/jar.age" --passphrase "$SCRATCH/wrong.txt" --json 2>&1) || rc=$?
CHECKS=$((CHECKS + 1))
((rc == 2)) || fail "import with the wrong passphrase exited $rc, not 2 (InvalidArgument)"
[[ $err == *passphrase* ]] || fail "the wrong-passphrase error does not name the field: $err"
rest2 GET /v1/profiles
assert_json '.profiles | map(select(.name == "jar")) | length == 0' "the failed import created nothing"

imported=$(brr2 profile import --file "$SCRATCH/jar.age" --passphrase "$SCRATCH/pp.txt" --json)
assert_eq "$(jq -r .profile.name <<<"$imported")" jar "imported profile name"

# The cookie came along: the browser of daemon 2 sends it.
rest2 POST /v1/builds/detect '{}'
assert_status 200 "second daemon detect"
rest2 POST /v1/profiles/jar/launch "$(jq -nc --arg u "http://127.0.0.1:$peer_port/echo" '{url: $u}')"
assert_status 200 "launch the imported Profile"
inst2=$(jq -r .instance.id <<<"$BODY")
tab2=$(jq -r .tab.id <<<"$BODY")
cookie_arrived() {
    rest2 GET "/v1/tabs/$tab2"
    [[ $(jq -r .title <<<"$BODY") == cookie:online ]]
}
wait_until cookie_arrived 30
rest2 POST "/v1/instances/$inst2/stop" '{}'

# The wrong passphrase over REST: 400 with the field. A profile archive's base64 is past
# the kernel's per-argument cap, so it goes through a file the way the oversized import does.
base64 -w0 "$SCRATCH/jar.age" >"$SCRATCH/jar.b64"
jq -nc --rawfile f "$SCRATCH/jar.b64" '{file: $f, passphrase: "wrong"}' >"$SCRATCH/wrong-import.json"
STATUS=$(curl -sS --max-time 120 --unix-socket "$D2_SOCK" -X POST -o "$SCRATCH/wrong-import.out" -w '%{http_code}' \
    -H 'content-type: application/json' --data-binary "@$SCRATCH/wrong-import.json" \
    "http://localhost/v1/profiles/import")
BODY=$(<"$SCRATCH/wrong-import.out")
assert_status 400 "wrong passphrase over REST"
assert_json '.code == "InvalidArgument" and .details.field == "passphrase"'
rm -f "$SCRATCH/jar.b64" "$SCRATCH/wrong-import.json" "$SCRATCH/wrong-import.out"

# --- an archive over the 48 MiB JSON limit: streamed by the CLI and by the raw routes
rest GET /v1/profiles/jar
jar_dir=$(jqv '.data_dir')
head -c $((60 * 1024 * 1024)) /dev/urandom >"$jar_dir/blob.bin"
brr profile export jar -o "$SCRATCH/big.age" --passphrase "$SCRATCH/pp.txt" >/dev/null
CHECKS=$((CHECKS + 1))
(($(stat -c %s "$SCRATCH/big.age") > 48 * 1024 * 1024)) || fail "the big archive is not over 48 MiB"
rm -f "$jar_dir/blob.bin"

base64 -w0 "$SCRATCH/big.age" >"$SCRATCH/big.b64"
jq -nc --rawfile f "$SCRATCH/big.b64" '{file: $f, passphrase: "irrelevant"}' >"$SCRATCH/big-request.json"
json_status=$(curl -sS --max-time 300 --unix-socket "$D2_SOCK" -X POST -o "$SCRATCH/json-import.json" -w '%{http_code}' \
    -H 'content-type: application/json' --data-binary "@$SCRATCH/big-request.json" "http://localhost/v1/profiles/import")
CHECKS=$((CHECKS + 1))
[[ $json_status == 413 ]] || fail "the JSON import answered $json_status, not 413"
CHECKS=$((CHECKS + 1))
[[ $(jq -r .code "$SCRATCH/json-import.json") == RequestTooLarge ]] || fail "the JSON import error is not RequestTooLarge"
rm -f "$SCRATCH/big.b64" "$SCRATCH/big-request.json"

pp_b64=$(base64 -w0 "$SCRATCH/pp.txt")
raw_status=$(curl -sS --max-time 300 --unix-socket "$D2_SOCK" -X POST -o "$SCRATCH/raw-import.json" -w '%{http_code}' \
    -H "X-Brr-Passphrase: $pp_b64" -H 'content-type: application/octet-stream' \
    --data-binary "@$SCRATCH/big.age" "http://localhost/v1/profiles/import/raw?name=big-raw")
CHECKS=$((CHECKS + 1))
[[ $raw_status == 200 ]] || fail "raw import answered $raw_status: $(<"$SCRATCH/raw-import.json")"
assert_eq "$(jq -r .profile.name "$SCRATCH/raw-import.json")" big-raw "raw-imported profile name"
raw_dir=$(jq -r .profile.data_dir "$SCRATCH/raw-import.json")
CHECKS=$((CHECKS + 1))
[[ $(stat -c %s "$raw_dir/blob.bin") == $((60 * 1024 * 1024)) ]] || fail "the big file did not arrive whole"

raw_status=$(curl -sS --max-time 300 --unix-socket "$D2_SOCK" -X POST -o "$SCRATCH/raw-wrong.json" -w '%{http_code}' \
    -H "X-Brr-Passphrase: $(printf wrong | base64 -w0)" -H 'content-type: application/octet-stream' \
    --data-binary "@$SCRATCH/big.age" "http://localhost/v1/profiles/import/raw")
CHECKS=$((CHECKS + 1))
[[ $raw_status == 400 ]] || fail "raw import with a wrong passphrase answered $raw_status"
CHECKS=$((CHECKS + 1))
[[ $(jq -r .details.field "$SCRATCH/raw-wrong.json") == passphrase ]] || fail "the raw wrong-passphrase error does not name the field"

imported=$(brr2 profile import --file "$SCRATCH/big.age" --passphrase "$SCRATCH/pp.txt" --name big-cli --json)
assert_eq "$(jq -r .profile.name <<<"$imported")" big-cli "CLI-imported big profile name"
CHECKS=$((CHECKS + 1))
[[ -z $(ls -A "$D2_DIR/data/brr/tmp" 2>/dev/null) ]] || fail "scratch files left under data/tmp"

# --- backups on local storage with keep 1
rest PATCH /v1/profiles/jar '{"backup_keep": 1}'
assert_status 200 "backup_keep 1"
rest POST /v1/profiles/jar/backups '{}'
assert_status 200 "backup.create"
assert_json '.backend == "local"' "a local backup"
sleep 1.1
rest POST /v1/profiles/jar/backups '{}'
assert_status 200 "second backup.create"
rest GET /v1/profiles/jar/backups
assert_json '.backups | length == 1' "keep 1 prunes the older backup"
rest GET "/v1/events?kinds=backup.completed"
assert_json '[.events[] | select(.kind == "backup.completed")] | length == 2' "two backup.completed events"

# --- backups on rustfs
if (need_s3) >/dev/null 2>&1; then
    rest PUT /v1/storage/backend "$(jq -nc '{backend: "s3", endpoint: "http://127.0.0.1:9900", region: "us-east-1",
        bucket: "brr-artifacts", access_key_id: "brradmin", secret_access_key: "brrsecret123", path_style: true}')"
    assert_status 200 "storage backend s3"
    for _ in 1 2; do
        rest POST /v1/profiles/jar/backups '{}'
        assert_status 200 "backup.create on s3"
        sleep 1.1
    done
    rest GET /v1/profiles/jar/backups
    assert_json '[.backups[] | select(.backend == "s3")] | length == 1' "one s3 backup after keep 1"
    s3_backup=$(jq -r '[.backups[] | select(.backend == "s3")][0].id' <<<"$BODY")
    rest POST "/v1/backups/$s3_backup/restore" '{"name": "from-s3"}'
    assert_status 200 "backup.restore from s3"
    launch from-s3 "" "http://127.0.0.1:$peer_port/echo"
    restored=$(jqv '.instance.id')
    tab3=$(jqv '.tab.id')
    wait_title "$tab3" cookie:online 30
    stop_and_wait "$restored"
    rest DELETE "/v1/backups/$s3_backup"
    assert_status 200 "backup.delete on s3"
    rest PUT /v1/storage/backend '{"backend": "local"}'
    assert_status 200 "storage back to local"
else
    echo "s3 part skipped: rustfs is not answering at $RUSTFS_URL"
fi

# --- schedule: the next minute, local time, in the daemon's own TZ
rest GET "/v1/events?kinds=backup.completed"
completed_before=$(jq '.events | length' <<<"$BODY")
at=$(date -d '+1 min' +%H:%M)
rest PATCH /v1/profiles/jar "$(jq -nc --arg s "daily@$at" '{backup_schedule: $s}')"
assert_status 200 "backup_schedule daily@$at"
assert_json ".backup_schedule == \"daily@$at\""
scheduled_done() {
    rest GET "/v1/events?kinds=backup.completed"
    [[ $(jq '.events | length' <<<"$BODY") -gt $completed_before ]]
}
wait_until scheduled_done 130

# The second daemon has no passphrase: a schedule is refused.
rest2 PATCH /v1/profiles/jar "$(jq -nc --arg s "daily@$at" '{backup_schedule: $s}')"
assert_status 400 "schedule without a passphrase"
assert_json '.code == "InvalidArgument" and .details.field == "backup_schedule"'

finish "passphrase secret, export/import to a fresh root with the cookie, wrong passphrase 400, keep 1, s3 backup and restore, scheduled backup"
