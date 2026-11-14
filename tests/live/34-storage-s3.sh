#!/usr/bin/env bash
# T5: Artifacts on S3-compatible storage (rustfs), presigned URLs, and the retention sweep deleting them.
source "$(dirname "$0")/lib.sh"
set -euo pipefail
need_chrome
need_rustfs

S3_SECRET=brrsecret123
backend() { # SECRET: the rustfs backend as JSON
    jq -nc --arg s "$1" '{backend: "s3", endpoint: "http://127.0.0.1:9900", region: "us-east-1", bucket: "brr-artifacts",
        access_key_id: "brradmin", secret_access_key: $s, path_style: true}'
}

detect_builds
http_server "$M4_FIXTURES"
base=http://127.0.0.1:$HTTP_PORT

# A wrong secret fails the connection test and the body never repeats it.
wrong=wrong-secret-$(ulid)
rest POST /v1/storage/backend/test "$(backend "$wrong")"
assert_status 503 "backend_test with a wrong secret"
assert_json '.code == "StorageUnavailable"'
CHECKS=$((CHECKS + 1))
[[ $BODY != *"$wrong"* ]] || fail "the error body repeats the secret"

rest POST /v1/storage/backend/test "$(backend "$S3_SECRET")"
assert_status 200 "backend_test with the right secret"

rest PUT /v1/storage/backend "$(backend "$S3_SECRET")"
assert_status 200 "backend_set s3"
rest GET /v1/storage/backend
assert_status 200 backend_get
assert_json '.backend == "s3" and .secret_access_key.set == true and (.secret_access_key.hint | endswith("t123"))' "the secret is hinted, not shown"
CHECKS=$((CHECKS + 1))
[[ $BODY != *"$S3_SECRET"* ]] || fail "backend_get repeats the secret"

# A result over the inline limit comes back as a presigned URL.
rest GET "/v1/pdf?url=$base/big-pdf.html"
assert_status 200 "big pdf one-shot"
assert_json '(.artifact_id | length > 0) and (.inline_base64 == null) and (.size > 20000000)' "a 20 MB+ PDF is not inline"
id=$(jqv '.artifact_id')
url=$(jqv '.url')
size=$(jqv '.size')
curl -sS --max-time 120 "$url" -o "$SCRATCH/big.pdf"
assert_eq "$(head -c 5 "$SCRATCH/big.pdf")" '%PDF-' "the presigned URL serves the PDF"
assert_eq "$(stat -c %s "$SCRATCH/big.pdf")" "$size" "presigned download size"
rest GET "/v1/artifacts/$id"
assert_json '.backend == "s3"' "the Artifact lives on s3"

# Age 0 makes it due; the dry run lists it, the real sweep deletes it from rustfs.
rest PUT /v1/retention '{"artifacts_max_age_days": 0}'
assert_status 200 retention.set
rest POST /v1/retention/sweep '{"dry_run": true}'
assert_status 200 "dry run"
assert_json "[.groups[] | select(.rule == \"artifacts_age\") | .items[] | select(.id == \"$id\")] | length == 1" "the dry run lists the Artifact under artifacts_age"
rest POST /v1/retention/sweep '{"dry_run": false}'
assert_status 200 "real sweep"
assert_json "[.groups[] | select(.rule == \"artifacts_age\") | .items[] | select(.id == \"$id\")] | length == 1"
assert_eq "$(curl -s -o /dev/null -w '%{http_code}' --max-time 30 "$url")" 404 "the presigned URL after the sweep"
rest GET "/v1/artifacts/$id"
assert_status 404 "artifact.get after the sweep"

finish "s3 backend on rustfs: wrong secret, hint, ${size}-byte presigned PDF, sweep deletes it"
