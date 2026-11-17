#!/usr/bin/env bash
# Pack target/release/brr with its licenses into dist/brr-VERSION-TARGET.tar.gz and a .sha256 beside it.
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:?usage: package.sh VERSION TARGET}"
target="${2:?usage: package.sh VERSION TARGET}"
name="brr-$version-$target"
out="dist/$name"

rm -rf "$out"
mkdir -p "$out/autoconsent" "$out/playwright"
cp target/release/brr LICENSE README.md deploy/NOTICE "$out/"
cp crates/brr-content/vendor/autoconsent/* "$out/autoconsent/"
cp crates/brr-engine/assets/playwright-LICENSE crates/brr-engine/assets/playwright-NOTICE "$out/playwright/"

tar -C dist -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "dist/$name.tar.gz"
