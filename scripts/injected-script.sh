#!/usr/bin/env bash
set -euo pipefail

PLAYWRIGHT_TAG=v1.62.1

root=$(cd "$(dirname "$0")/.." && pwd)
assets="$root/crates/brr-engine/assets"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

src="$tmp/playwright-${PLAYWRIGHT_TAG#v}"
curl -fsSL "https://codeload.github.com/microsoft/playwright/tar.gz/refs/tags/$PLAYWRIGHT_TAG" | tar -xz -C "$tmp"

mkdir -p "$assets"
cat >"$tmp/brr-entry.ts" <<ENTRY
import { InjectedScript } from '$src/packages/injected/src/injectedScript';
(globalThis as any).__brrInjectedScript = InjectedScript;
ENTRY

# The @isomorphic/* and @protocol/* aliases resolve through Playwright's tsconfig.json, found from
# the sources. The plugin turns `x.css?inline` imports into strings, as Playwright's own
# inlineCSSPlugin (utils/generate_injected.js) does; the bun CLI has no way to do that.
bun -e 'import path from "node:path";
const [entry, output] = process.argv.slice(1);
const inlineCss = {
    name: "inline-css",
    setup(build) {
        build.onResolve({ filter: /\.css\?inline$/ }, (args) => ({
            path: path.resolve(path.dirname(args.importer), args.path.replace(/\?inline$/, "")),
        }));
        build.onLoad({ filter: /\.css$/ }, async (args) => ({
            loader: "text",
            contents: await Bun.file(args.path).text(),
        }));
    },
};
const result = await Bun.build({ entrypoints: [entry], format: "iife", target: "browser", minify: true, plugins: [inlineCss] });
if (!result.success) {
    console.error(result.logs.join("\n"));
    process.exit(1);
}
await Bun.write(output, await result.outputs[0].text());' "$tmp/brr-entry.ts" "$tmp/injected.js"
{
    echo "// Playwright $PLAYWRIGHT_TAG injected script, Apache-2.0, see NOTICE"
    echo "// Built with bun $(bun --version)."
    sed 's|^|// |; s| *$||' "$src/NOTICE"
    cat "$tmp/injected.js"
} >"$assets/injected.js"

cp -f "$src/LICENSE" "$assets/playwright-LICENSE"
cp -f "$src/NOTICE" "$assets/playwright-NOTICE"
cp -f "$src/packages/isomorphic/deviceDescriptorsSource.json" "$assets/devices.json"

bun -e 'const [input, output] = process.argv.slice(1);
const { USKeyboardLayout } = await import(input);
await Bun.write(output, JSON.stringify(USKeyboardLayout));' \
    "$src/packages/playwright-core/src/server/usKeyboardLayout.ts" "$assets/keyboard-layout.json"

(cd "$assets" && sha256sum injected.js devices.json keyboard-layout.json playwright-NOTICE playwright-LICENSE)
