#!/usr/bin/env bash
# P7: the free-disk floor, the per-Profile quota and Download limits.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
mkdir -p "$SCRATCH/site"
head -c 3000000 /dev/urandom >"$SCRATCH/site/attachment.bin"
echo '<title>site</title>' >"$SCRATCH/site/index.html"
http_server "$SCRATCH/site" --download
base=http://127.0.0.1:$HTTP_PORT
new_profile eph ephemeral

# 1. A floor above the free space refuses launches.
rest PUT /v1/retention '{"free_disk_floor_gb": 1000000}'
assert_status 200 "floor above free space"
rest POST /v1/profiles/eph/launch '{}'
assert_status 507 "launch below the disk floor"
assert_json '.code == "QuotaExceeded" and .details.scope == "disk_floor"'
rest PUT /v1/retention '{"free_disk_floor_gb": 1}'
assert_status 200 "floor back to normal"

# 2. A Profile over its disk quota refuses its next launch.
new_profile pers persistent
rest PATCH /v1/profiles/pers '{"disk_quota_bytes": 1}'
assert_status 200 "set the quota"
launch pers "" "$base/index.html"
first=$(jqv '.instance.id')
stop_instance "$first"
wait_until '[[ $(instance_state "$first") == stopped ]]' 15
rest POST /v1/profiles/pers/launch '{}'
assert_status 507 "launch over the profile quota"
assert_json '.code == "QuotaExceeded" and .details.scope == "profile"'

# 3. Downloads: over the size limit they are cancelled, under it they become Artifacts.
rest PUT /v1/retention '{"download_max_mb": 1}'
assert_status 200 "download limit 1 MB"
launch eph "" "$base/index.html"
instance=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
download() { # runs the navigation that turns into a download; its own status does not matter
    tab_call "$tab" navigate "{\"url\": \"$base/attachment.bin\", \"timeout\": 15000}" || true
}
event_with() { # KIND JQ: the Instance's events hold one of KIND satisfying JQ
    rest GET "/v1/events?kinds=$1&instance_id=$instance&limit=100"
    jq -e "[.events[] | select($2)] | length >= 1" >/dev/null <<<"$BODY"
}
download
wait_until "event_with download.cancelled '.data.reason == \"too_large\"'" 30
rest GET '/v1/artifacts?kind=download'
assert_json '.artifacts == []' "a cancelled download leaves no Artifact"

rest PUT /v1/retention '{"download_max_mb": 10}'
assert_status 200 "download limit 10 MB"
download
wait_until "event_with download.completed 'true'" 30
rest GET '/v1/artifacts?kind=download'
assert_json '.artifacts | length == 1' "the completed download is an Artifact"
assert_json '.artifacts[0].size_bytes == 3000000' "with all its bytes"

# 4. A floor above free space also stops Downloads of a running Instance.
rest PUT /v1/retention '{"free_disk_floor_gb": 1000000}'
assert_status 200 "floor above free space again"
download
wait_until "event_with download.cancelled '.data.reason == \"disk_floor\"'" 30
rest GET '/v1/artifacts?kind=download'
assert_json '.artifacts | length == 1' "no new Artifact under the floor"

rest PUT /v1/retention '{"free_disk_floor_gb": 1}'
stop_instance "$instance"
finish "disk floor, profile quota, download size limit and floor during a download"
