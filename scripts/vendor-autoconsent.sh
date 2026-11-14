#!/usr/bin/env bash
set -euo pipefail

version=${1:-18.3.0}

root=$(cd "$(dirname "$0")/.." && pwd)
dest="$root/crates/brr-content/vendor/autoconsent"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL "https://registry.npmjs.org/@duckduckgo/autoconsent/-/autoconsent-$version.tgz" | tar xz -C "$work"

mkdir -p "$dest"
cp -f "$work/package/dist/autoconsent.standalone.js" "$dest/autoconsent.standalone.js"
cp -f "$work/package/LICENSE" "$dest/LICENSE"
printf '%s\n' "$version" >"$dest/VERSION"
