#!/usr/bin/env bash
source "$(dirname "$0")/lib.sh"
set -euo pipefail

rest GET /v1/templates
assert_status 200 template.list
assert_json '[.templates[] | select(.builtin)] | length == 4' "4 built-in templates"

new_profile pers persistent
assert_json '.kind == "persistent" and .name == "pers"'
new_profile eph ephemeral
assert_json '.kind == "ephemeral"'
rest GET /v1/profiles
assert_json '[.profiles[].name] | index("pers") and index("eph")' "both listed"

rest POST /v1/profiles '{"name": "pers", "template": "chromium-headless"}'
assert_status 409 "duplicate name"
assert_json '.code == "Conflict" and .details.reason == "name_taken"'

rest DELETE /v1/templates/chromium-headless
assert_status 409 "delete built-in"
assert_json '.code == "Conflict" and .details.reason == "builtin"'

rest GET /v1/profiles/pers
settings=$(jq -c '.settings' <<<"$BODY")
rest PATCH /v1/profiles/pers "$(jq -nc --argjson s "$settings" '{settings: ($s | .args = ["--remote-debugging-port=1"])}')"
assert_status 400 "managed switch in args"
assert_json '.code == "InvalidArgument" and .details.field == "args"'

rest PATCH /v1/profiles/pers "$(jq -nc --argjson s "$settings" '{settings: ($s | .args = ["--lang=fr"])}')"
assert_status 200 "allowed args"
assert_json '.settings.args == ["--lang=fr"]'

rest POST /v1/profiles/pers/save-as-template '{"name": "mine"}'
assert_status 200 "save-as-template"
assert_json '.name == "mine" and .builtin == false'
assert_json '.settings.args == ["--lang=fr"]' "template copies the profile's settings"

new_profile fromtpl persistent mine
assert_json '.settings.args == ["--lang=fr"]' "profile copies the template's settings"

rest GET /v1/templates/mine
rest PATCH /v1/templates/mine "$(jq -c '{settings: (.settings | .args = ["--lang=de"])}' <<<"$BODY")"
assert_status 200 "update template"
assert_json '.settings.args == ["--lang=de"]'

rest GET /v1/profiles/fromtpl
assert_json '.settings.args == ["--lang=fr"]' "existing profile keeps its old settings"
new_profile fromtpl2 persistent mine
assert_json '.settings.args == ["--lang=de"]' "new profile sees the updated template"

rest DELETE /v1/profiles/eph
assert_status 200 "delete profile"
rest GET /v1/profiles/eph
assert_status 404 "deleted profile is gone"

finish "built-ins, kinds, conflicts, managed args, templates copy by value"
