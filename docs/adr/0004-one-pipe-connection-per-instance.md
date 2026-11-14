# One pipe connection per Instance, multiplexed by brr

Each managed Chromium-family Instance is launched with `--remote-debugging-pipe`, and brr owns the only browser connection. Every Client, including `/cdp` WebSocket users (Playwright, Puppeteer), is a downstream session that brr multiplexes onto it with flat sessions. We rejected a byte-level WebSocket proxy to `--remote-debugging-port`: it cannot enforce Leases, cannot keep brr's own download path (clients overwrite it with `Browser.setDownloadBehavior`), and sees `Browser.close` only as a crash. brr has its own thin CDP client (chromiumoxide has no pipe transport).

## Consequences
- A `/cdp` connection takes an exclusive Instance Lease and sees only that Instance's targets. brr intercepts `Browser.close` (it stops the Instance through the state machine, trigger `stop`) and `Browser.setDownloadBehavior` (brr keeps its own download dir).
- A closed pipe makes the browser exit, so a restarted daemon cannot adopt Instances. Reconciliation kills leftovers by environment marker (`BRR_INSTANCE_ID`) instead.
- The multiplexer is brr's own code: message `id` rewriting, `sessionId` routing, and 256 MiB WebSocket message limits.
