#!/usr/bin/env bash
set -euo pipefail

CAMOUFOX_TAG=v158.0.1-beta.41

root=$(cd "$(dirname "$0")/.." && pwd)
fixtures="$root/crates/brr-camoufox/tests/fixtures"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL "https://codeload.github.com/daijro/camoufox/tar.gz/refs/tags/$CAMOUFOX_TAG" | tar xz -C "$work" --strip-components=1
cd "$work"

python3 -m venv .venv
.venv/bin/pip install -q -r ci/requirements.txt -e python
.venv/bin/python python/scripts/pin-fpgen-model.py
.venv/bin/python typescript/scripts/golden/identity_golden.py
.venv/bin/python typescript/scripts/golden/launch_golden.py

rm -rf "$fixtures"
mkdir -p "$fixtures"
cp -rf typescript/tests/fixtures/identity typescript/tests/fixtures/launch "$fixtures/"
# flate2 is not an approved crate, so the recorded gzip members are stored plain.
find "$fixtures" -name '*.json.gz' -exec gunzip -f {} +

model_dir=$(.venv/bin/python -c 'from camoufox.fpgen_model import fpgen_data_dir; print(fpgen_data_dir())')
mkdir -p "$HOME/.cache/brr-test"
rm -rf "$HOME/.cache/brr-test/fpgen-model"
cp -rf "$model_dir" "$HOME/.cache/brr-test/fpgen-model"
# Model::open only reads, so the model is unpacked here
zstd -dqf "$HOME/.cache/brr-test/fpgen-model/values.dat.zst" -o "$HOME/.cache/brr-test/fpgen-model/values.dat"
