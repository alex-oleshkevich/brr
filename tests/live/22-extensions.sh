#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome
command -v zip >/dev/null && command -v python3 >/dev/null || skip "zip and python3 are needed to build the test archives"

detect_builds
peer_port=$(free_port)
peer_server "$peer_port"
PAGE_URL=http://127.0.0.1:$peer_port/

# enable_extensions PROFILE EXTENSION_ID...: profile.extensions_set; sets $STATUS and $BODY.
enable_extensions() {
    local profile=$1
    shift
    rest PUT "/v1/profiles/$profile/extensions" "$(jq -nc '{extensions: [$ARGS.positional[] | {extension_id: ., auto_update: false}]}' --args "$@")"
}

# 1. A local folder goes up as a tar stream and registers for Chromium.
ext_dir=$SCRATCH/ext
make_ext "$ext_dir"
out=$(brr extension add --file "$ext_dir" --json)
BODY=$out
assert_json '.engine == "chromium" and .source == "local" and (.versions | length == 1)' "folder extension"
ext_id=$(jqv '.id')

# 2. Enabled on a Profile, its content script runs in a launched page.
new_profile with-ext ephemeral
enable_extensions with-ext "$ext_id"
assert_status 200 profile.extensions_set
launch with-ext "" "$PAGE_URL"
inst=$(jqv '.instance.id')
tab=$(jqv '.tab.id')
wait_title "$tab" "ext-ok" 30
stop_instance "$inst"
wait_until '[[ $(instance_state "$inst") == stopped ]]' 15

# 3. A Firefox add-on registers for Camoufox and cannot be enabled on a Chromium profile.
xpi_dir=$SCRATCH/xpi
mkdir -p "$xpi_dir"
cat >"$xpi_dir/manifest.json" <<'JSON'
{"manifest_version": 2, "name": "gecko ext", "version": "1.0",
 "browser_specific_settings": {"gecko": {"id": "live@brr.test"}}}
JSON
(cd "$xpi_dir" && zip -q ../live.xpi manifest.json)
BODY=$(brr extension add --file "$SCRATCH/live.xpi" --json)
assert_json '.engine == "camoufox"' ".xpi is a Camoufox extension"
xpi_id=$(jqv '.id')
enable_extensions with-ext "$xpi_id"
assert_status 400 "a Camoufox extension on a Chromium profile"
assert_json '.code == "InvalidArgument" and .details.field == "extensions"'

# 4. A tar entry that climbs out of its folder is refused.
python3 - "$SCRATCH/evil.tar" <<'PY'
import io, sys, tarfile
with tarfile.open(sys.argv[1], "w") as tar:
    data = b"{}"
    info = tarfile.TarInfo("../x")
    info.size = len(data)
    tar.addfile(info, io.BytesIO(data))
PY
rest POST /v1/extensions "$(jq -nc --rawfile f <(base64 -w0 "$SCRATCH/evil.tar") '{file: $f}')"
assert_status 400 "tar with a ../ entry"
assert_json '.code == "InvalidArgument"'

if [[ ${BRR_LIVE_NETWORK-} != 1 ]]; then
    finish "folder tar upload, content script runs, xpi engine, bad tar; Web Store part skipped (BRR_LIVE_NETWORK unset)"
    exit 0
fi

# With network: a public Web Store id, update is a no-op, headless_shell cannot load extensions.
rest POST /v1/extensions '{"source": "cws", "ref": "ddkjiahejlhfcafbddmgiahcphecmpfh"}'
assert_status 200 "add uBlock Origin Lite from the Web Store"
cws_id=$(jqv '.id')
version_dir=$(jqv '.versions[0].path')
CHECKS=$((CHECKS + 1))
[[ -d $version_dir ]] || fail "version dir missing: $version_dir"
rest POST "/v1/extensions/$cws_id/update" '{}'
assert_status 200 extension.update
assert_json '.updated == false' "already the latest version"

rest POST /v1/builds '{"source": "headless_shell", "channel": "stable"}'
assert_status 200 "install headless_shell"
shell_id=$(jqv '.id')
wait_until 'rest GET "/v1/builds/$shell_id"; [[ $(jq -r .state <<<"$BODY") == installed ]]' 600
new_profile shell-profile persistent
profile_set shell-profile '.build = {build_id: $b}' --arg b "$shell_id"
enable_extensions shell-profile "$cws_id"
assert_status 422 "extensions on headless_shell"
assert_json '.code == "EngineUnsupported" and .details.capability == "unpacked_extensions"'

finish "folder tar upload, content script runs, xpi engine, bad tar, Web Store, EngineUnsupported"
