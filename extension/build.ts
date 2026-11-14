import { cp, mkdir, rm } from "node:fs/promises";
import { join } from "node:path";

const dist = join(import.meta.dir, "dist");
await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });

const result = await Bun.build({
  entrypoints: [join(import.meta.dir, "src/background.ts"), join(import.meta.dir, "src/popup.ts")],
  outdir: dist,
  target: "browser",
  format: "esm",
  minify: false,
});
if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}

for (const file of ["manifest.json", "popup.html", "popup.css", "icons"])
  await cp(join(import.meta.dir, file), join(dist, file), { recursive: true });
