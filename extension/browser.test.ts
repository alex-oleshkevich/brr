import { expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const chromeBin = process.env.BRR_TEST_CHROME;
if (!chromeBin) console.log("browser.test.ts skipped: set BRR_TEST_CHROME to a Chrome for Testing binary");

// Minimal browser-level CDP client over Bun's WebSocket.
class Cdp {
  private nextId = 1;
  private pending = new Map<number, (m: any) => void>();
  private constructor(private ws: WebSocket) {
    ws.onmessage = (event) => {
      const message = JSON.parse(String(event.data));
      this.pending.get(message.id)?.(message);
    };
  }
  static open(url: string): Promise<Cdp> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(url);
      ws.onopen = () => resolve(new Cdp(ws));
      ws.onerror = () => reject(new Error(`cannot connect to ${url}`));
    });
  }
  send(method: string, params: object = {}, sessionId?: string): Promise<any> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, (m) => (m.error ? reject(new Error(m.error.message)) : resolve(m.result)));
      this.ws.send(JSON.stringify({ id, method, params, sessionId }));
    });
  }
  close() {
    this.ws.close();
  }
}

async function until<T>(what: string, f: () => Promise<T | undefined>, ms = 15000): Promise<T> {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    const value = await f().catch(() => undefined);
    if (value !== undefined) return value;
    await Bun.sleep(100);
  }
  throw new Error(`timed out waiting for ${what}`);
}

test.skipIf(!chromeBin)("the popup shows the consent text for the default relay URL", async () => {
  const profile = mkdtempSync(join(tmpdir(), "brr-ext-"));
  const dist = join(import.meta.dir, "dist");
  const chrome = Bun.spawn(
    [
      chromeBin!,
      "--headless=new",
      "--no-sandbox",
      `--load-extension=${dist}`,
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "about:blank",
    ],
    { stdout: "ignore", stderr: "ignore" },
  );
  let cdp: Cdp | undefined;
  try {
    const [port, path] = await until("DevToolsActivePort", async () =>
      readFileSync(join(profile, "DevToolsActivePort"), "utf8").trim().split("\n"),
    );
    cdp = await Cdp.open(`ws://127.0.0.1:${port}${path}`);

    // Component extensions also run a background.js worker; ours reports the manifest name.
    const extensionId = await until("extension service worker", async () => {
      const { targetInfos } = await cdp!.send("Target.getTargets");
      for (const t of targetInfos) {
        const id = t.type === "service_worker" && /^chrome-extension:\/\/([a-p]{32})\/background\.js$/.exec(t.url)?.[1];
        if (!id) continue;
        const { sessionId } = await cdp!.send("Target.attachToTarget", { targetId: t.targetId, flatten: true });
        const { result } = await cdp!.send(
          "Runtime.evaluate",
          { expression: "chrome.runtime.getManifest().name" },
          sessionId,
        );
        if (result.value === "brr companion") return id;
      }
    });

    const { targetId } = await cdp.send("Target.createTarget", { url: `chrome-extension://${extensionId}/popup.html` });
    const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
    const text = await until("popup text", async () => {
      const { result } = await cdp!.send("Runtime.evaluate", { expression: "document.body.innerText" }, sessionId);
      return result.value?.includes("Allow brr at") ? (result.value as string) : undefined;
    });
    expect(text).toContain("Allow brr at ws://127.0.0.1:9898/v1/extension-relay");
  } finally {
    cdp?.close();
    chrome.kill();
    await chrome.exited;
    rmSync(profile, { recursive: true, force: true });
  }
}, 60000);
