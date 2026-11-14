// Raw CDP client over Bun's built-in WebSocket. Usage: bun cdp-client.ts WS_URL SCRIPT_JSON
// SCRIPT_JSON is an array of {method, params?, sessionId?} (sent in order, each waiting for its
// response), {sleep_ms}, or {hold_ms} (wait with the socket open). Every frame is printed as one
// JSON line; the process exits when the script ends or the socket closes.
const [url, scriptText] = process.argv.slice(2);
const script: Array<Record<string, unknown>> = JSON.parse(scriptText);
const socket = new WebSocket(url);
const waiting = new Map<number, () => void>();
let nextId = 0;

const out = (frame: unknown) => console.log(JSON.stringify(frame));
const finish = (code: number) => process.exit(code);

socket.onmessage = (event) => {
  const frame = JSON.parse(String(event.data));
  out(frame);
  if (typeof frame.id === "number") waiting.get(frame.id)?.();
};
socket.onclose = () => {
  out({ closed: true });
  finish(0);
};
socket.onerror = () => {
  out({ error: "websocket error" });
  finish(1);
};

socket.onopen = async () => {
  for (const step of script) {
    if (typeof step.sleep_ms === "number" || typeof step.hold_ms === "number") {
      await new Promise((resolve) => setTimeout(resolve, Number(step.sleep_ms ?? step.hold_ms)));
      continue;
    }
    const id = ++nextId;
    const answered = new Promise<void>((resolve) => waiting.set(id, resolve));
    socket.send(JSON.stringify({ id, method: step.method, params: step.params ?? {}, sessionId: step.sessionId }));
    // Browser.close answers and then the socket closes; do not wait forever for either.
    await Promise.race([answered, new Promise((resolve) => setTimeout(resolve, 15000))]);
  }
  socket.close();
  setTimeout(() => finish(0), 2000);
};
