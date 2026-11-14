#!/usr/bin/env bash
set -euo pipefail

CAMOUFOX_TAG=v158.0.1-beta.41

root=$(cd "$(dirname "$0")/.." && pwd)
data="$root/crates/brr-camoufox/data"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

curl -fsSL "https://codeload.github.com/daijro/camoufox/tar.gz/refs/tags/$CAMOUFOX_TAG" | tar xz -C "$work" --strip-components=1
src="$work/python/src/camoufox"

rm -rf "$data"
mkdir -p "$data"

for file in essential-fonts fingerprint-presets fingerprint-presets-v150 font-bases font-groups fonts \
    launcher-constants media-devices voice-manifests voice-uris fpgen-model; do
    cp -f "$src/$file.json" "$data/$file.json"
done
cp -f "$work/LICENSE" "$data/camoufox-LICENSE"

# No YAML or XML crate in brr: the Rust side reads JSON only.
for file in fpgen; do
    bun -e 'const [input, output] = process.argv.slice(1);
await Bun.write(output, JSON.stringify(Bun.YAML.parse(await Bun.file(input).text()), null, 1) + "\n");' \
        "$src/$file.yml" "$data/$file.json"
done

# Fields typescript/src/locales.ts reads, in document order.
bun -e 'const [input, output] = process.argv.slice(1);
const xml = await Bun.file(input).text();
const attrs = (tag) => Object.fromEntries([...tag.matchAll(/(\w+)="([^"]*)"/g)].map((m) => [m[1], m[2]]));
const territories = [];
for (const m of xml.matchAll(/<territory\s([^>]*)>([\s\S]*?)<\/territory>/g)) {
  const t = attrs(m[1]);
  const languages = [...m[2].matchAll(/<languagePopulation\s([^>]*?)\/?>/g)].map((l) => {
    const a = attrs(l[1]);
    return a.officialStatus === undefined
      ? { type: a.type, populationPercent: Number(a.populationPercent ?? 0) }
      : { type: a.type, populationPercent: Number(a.populationPercent ?? 0), officialStatus: a.officialStatus };
  });
  territories.push({ type: t.type, population: Number(t.population ?? 0), literacyPercent: Number(t.literacyPercent ?? 0), languages });
}
await Bun.write(output, JSON.stringify(territories, null, 1) + "\n");' \
    "$src/territoryInfo.xml" "$data/territory-info.json"

(cd "$data" && sha256sum -- * | sort -k2)
