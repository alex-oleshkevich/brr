#!/usr/bin/env bash
# T5: a large result becomes an Artifact on local storage, served by an unguessable URL.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome

detect_builds
http_server "$M4_FIXTURES"
base=http://127.0.0.1:$HTTP_PORT

rest GET "/v1/pdf?url=$base/article.html&return=url"
assert_status 200 "pdf as an Artifact"
assert_json '(.artifact_id | length > 0) and (.mime == "application/pdf") and (.url | contains("/v1/a/"))'
id=$(jqv '.artifact_id')
url=$(jqv '.url')
size=$(jqv '.size')

curl -sS --max-time 30 "$url" -o "$SCRATCH/a.pdf"
assert_eq "$(head -c 5 "$SCRATCH/a.pdf")" '%PDF-' "the public URL serves the PDF"
assert_eq "$(stat -c %s "$SCRATCH/a.pdf")" "$size" "served size"

rest GET '/v1/artifacts?kind=pdf'
assert_status 200 artifact.list
assert_json ".artifacts | map(.id) | index(\"$id\")" "listed under kind=pdf"
assert_json "[.artifacts[] | select(.id == \"$id\")][0].backend == \"local\""
rest GET '/v1/artifacts?kind=screenshot'
assert_json ".artifacts | map(.id) | index(\"$id\") | not" "not listed under another kind"

rest GET "/v1/artifacts/$id"
assert_status 200 artifact.get
assert_json ".id == \"$id\" and .kind == \"pdf\""

rest GET "/v1/artifacts/$id/fetch?return=inline"
assert_status 200 "artifact.fetch inline"
assert_eq "$(jqv '.inline_base64' | base64 -d | head -c 5)" '%PDF-' "inline bytes"

brr artifact fetch "$id" --return inline -o "$SCRATCH/f.pdf" >/dev/null
assert_eq "$(head -c 5 "$SCRATCH/f.pdf")" '%PDF-' "brr artifact fetch -o"
assert_eq "$(stat -c %s "$SCRATCH/f.pdf")" "$size" "CLI fetched size"

# A URL result has no bytes in the answer; the CLI fetches them for -o.
brr artifact fetch "$id" --return url -o "$SCRATCH/u.pdf" >/dev/null
assert_eq "$(head -c 5 "$SCRATCH/u.pdf")" '%PDF-' "brr artifact fetch --return url -o"
assert_eq "$(stat -c %s "$SCRATCH/u.pdf")" "$size" "CLI url-fetched size"

rest DELETE "/v1/artifacts/$id"
assert_status 200 artifact.delete
assert_eq "$(curl -s -o /dev/null -w '%{http_code}' --max-time 30 "$url")" 404 "the URL after delete"
rest GET "/v1/artifacts/$id"
assert_status 404 "artifact.get after delete"
assert_json '.code == "NotFound"'
rest DELETE "/v1/artifacts/$id"
assert_status 404 "deleting twice"

finish "pdf Artifact on local storage: URL, list, fetch inline and via CLI, delete"
