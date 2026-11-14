import { beforeEach, describe, expect, test } from "bun:test";
import { RelayConnection } from "./relayConnection";

const GROUP = 7;

class FakeEvent {
  listeners: Array<(...args: any[]) => void> = [];
  addListener(f: (...args: any[]) => void) {
    this.listeners.push(f);
  }
  removeListener(f: (...args: any[]) => void) {
    this.listeners = this.listeners.filter((l) => l !== f);
  }
  emit(...args: unknown[]) {
    for (const l of this.listeners) l(...args);
  }
}

class FakeSocket {
  sent: any[] = [];
  closed?: [number | undefined, string | undefined];
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  send(data: string) {
    this.sent.push(JSON.parse(data));
  }
  close(code?: number, reason?: string) {
    this.closed = [code, reason];
  }
  methods() {
    return this.sent.map((m) => m.method);
  }
  // Delivers a relay command and waits for its reply.
  async command(id: number, method: string, params: unknown[]) {
    this.onmessage!({ data: JSON.stringify({ id, method, params }) });
    await new Promise((r) => setTimeout(r, 0));
    return this.sent.find((m) => m.id === id);
  }
}

const tab = (id: number, over: Record<string, unknown> = {}) => ({
  id,
  groupId: GROUP,
  windowId: 1,
  url: `https://example.com/${id}`,
  ...over,
});

let events: Record<string, FakeEvent>;
let calls: Array<[string, unknown[]]>;
let groupTabs: any[];
let ws: FakeSocket;

function installChrome() {
  events = Object.fromEntries(
    ["tabs.onCreated", "tabs.onUpdated", "tabs.onRemoved", "debugger.onEvent", "debugger.onDetach"].map((n) => [
      n,
      new FakeEvent(),
    ]),
  );
  calls = [];
  const record =
    (name: string, result: unknown = undefined) =>
    async (...args: unknown[]) => {
      calls.push([name, args]);
      return result;
    };
  (globalThis as any).chrome = {
    runtime: { getManifest: () => ({ version: "0.1.0" }) },
    tabs: {
      query: async () => groupTabs,
      group: record("tabs.group", GROUP),
      create: record("tabs.create", tab(9, { groupId: -1, url: "" })),
      remove: record("tabs.remove"),
      update: record("tabs.update"),
      onCreated: events["tabs.onCreated"],
      onUpdated: events["tabs.onUpdated"],
      onRemoved: events["tabs.onRemoved"],
    },
    debugger: {
      attach: record("debugger.attach"),
      detach: record("debugger.detach"),
      sendCommand: record("debugger.sendCommand", { result: { value: 3 } }),
      onEvent: events["debugger.onEvent"],
      onDetach: events["debugger.onDetach"],
    },
  };
}

async function connect(): Promise<RelayConnection> {
  const conn = new RelayConnection(ws, GROUP);
  await conn.start();
  ws.sent.length = 0;
  return conn;
}

beforeEach(() => {
  groupTabs = [tab(5), tab(6)];
  ws = new FakeSocket();
  installChrome();
});

describe("relay connection", () => {
  test("handshake order", async () => {
    const conn = new RelayConnection(ws, GROUP);
    await conn.start();
    expect(ws.methods()).toEqual([
      "extension.hello",
      "chrome.tabs.onCreated",
      "chrome.tabs.onCreated",
      "extension.initialized",
    ]);
    expect(ws.sent[0].params[0]).toMatchObject({ protocol: 1, version: "0.1.0" });
    expect(ws.sent.slice(1, 3).map((m) => m.params[0].id)).toEqual([5, 6]);
  });

  test("allow list", async () => {
    await connect();
    const ok = await ws.command(1, "chrome.debugger.sendCommand", [{ tabId: 5 }, "Runtime.evaluate", { expression: "1+2" }]);
    expect(ok).toEqual({ id: 1, result: { result: { value: 3 } } });
    expect(calls).toEqual([["debugger.sendCommand", [{ tabId: 5 }, "Runtime.evaluate", { expression: "1+2" }]]]);

    expect(await ws.command(2, "chrome.cookies.getAll", [{}])).toEqual({ id: 2, error: "command not allowed" });
    expect(await ws.command(3, "chrome.debugger.sendCommand", [{ tabId: 99 }, "Runtime.evaluate", {}])).toEqual({
      id: 3,
      error: "tab 99 is not in the brr tab group",
    });
  });

  test("chrome errors are returned, not thrown", async () => {
    await connect();
    (globalThis as any).chrome.debugger.attach = async () => {
      throw new Error("Another debugger is already attached");
    };
    expect(await ws.command(1, "chrome.debugger.attach", [{ tabId: 5 }, "1.3"])).toEqual({
      id: 1,
      error: "Another debugger is already attached",
    });
  });

  test("created tabs join the group before the reply", async () => {
    await connect();
    const reply = await ws.command(1, "chrome.tabs.create", [{ url: "https://x.test", active: false }]);
    expect(reply.result.id).toBe(9);
    expect(calls).toContainEqual(["tabs.group", [{ tabIds: [9], groupId: GROUP }]]);
    expect(ws.methods()).toEqual(["chrome.tabs.onCreated", undefined]);
    expect(ws.sent[0].params[0].id).toBe(9);
  });

  test("group failure after create removes the tab", async () => {
    await connect();
    (globalThis as any).chrome.tabs.group = async () => {
      throw new Error("Tabs cannot be edited right now");
    };
    const reply = await ws.command(1, "chrome.tabs.create", [{ url: "https://x.test" }]);
    expect(reply).toEqual({ id: 1, error: "Tabs cannot be edited right now" });
    expect(calls).toContainEqual(["tabs.remove", [9]]);
  });

  test("group filter", async () => {
    const conn = await connect();
    events["debugger.onEvent"].emit({ tabId: 42 }, "Page.loadEventFired", {});
    events["debugger.onDetach"].emit({ tabId: 42 }, "canceled_by_user");
    events["tabs.onRemoved"].emit(42);
    expect(ws.sent).toEqual([]);

    events["debugger.onEvent"].emit({ tabId: 5 }, "Page.loadEventFired", { a: 1 });
    expect(ws.sent.pop()).toEqual({
      method: "chrome.debugger.onEvent",
      params: [{ tabId: 5 }, "Page.loadEventFired", { a: 1 }],
    });

    // tab 5 leaves the group
    events["tabs.onUpdated"].emit(5, { groupId: -1 }, tab(5, { groupId: -1 }));
    expect(calls).toEqual([["debugger.detach", [{ tabId: 5 }]]]);
    expect(ws.sent).toEqual([
      { method: "chrome.debugger.onDetach", params: [{ tabId: 5 }, "target_closed"] },
      { method: "chrome.tabs.onRemoved", params: [5] },
    ]);
    expect([...conn.members]).toEqual([6]);

    // a closed member is reported once
    ws.sent.length = 0;
    events["tabs.onRemoved"].emit(6);
    expect(ws.methods()).toEqual(["chrome.debugger.onDetach", "chrome.tabs.onRemoved"]);
    expect(ws.sent[0].params).toEqual([{ tabId: 6 }, "target_closed"]);
  });

  test("tabs joining the group and url/title updates", async () => {
    await connect();
    events["tabs.onUpdated"].emit(8, { groupId: GROUP }, tab(8));
    expect(ws.methods()).toEqual(["chrome.tabs.onCreated"]);
    ws.sent.length = 0;

    events["tabs.onUpdated"].emit(8, { status: "loading" }, tab(8));
    events["tabs.onUpdated"].emit(10, { title: "x" }, tab(10, { groupId: -1 }));
    expect(ws.sent).toEqual([]);
    events["tabs.onUpdated"].emit(8, { title: "new" }, tab(8, { title: "new" }));
    expect(ws.methods()).toEqual(["chrome.tabs.onUpdated"]);
  });

  test("chrome:// tabs are not forwarded and are reported as blocked", async () => {
    const conn = await connect();
    events["tabs.onCreated"].emit(tab(11, { url: "chrome://settings" }));
    events["tabs.onCreated"].emit(tab(12, { url: "devtools://devtools/bundled/x.html" }));
    expect(ws.sent).toEqual([]);
    expect([...conn.blocked]).toEqual([11, 12]);
    expect(conn.members.has(11)).toBe(false);
  });

  test("user-cancelled detach is forwarded", async () => {
    await connect();
    events["debugger.onDetach"].emit({ tabId: 6 }, "canceled_by_user");
    expect(ws.sent).toEqual([{ method: "chrome.debugger.onDetach", params: [{ tabId: 6 }, "canceled_by_user"] }]);
  });

  test("an attach that completes after the tab left the group is undone", async () => {
    await connect();
    (globalThis as any).chrome.debugger.attach = async () => {
      events["tabs.onUpdated"].emit(5, { groupId: -1 }, tab(5, { groupId: -1 }));
    };
    calls.length = 0;
    await ws.command(1, "chrome.debugger.attach", [{ tabId: 5 }, "1.3"]);
    // Once from leaving the group and once for the attach that landed afterwards.
    expect(calls.filter(([name]) => name === "debugger.detach")).toHaveLength(2);
  });

  test("closing detaches attached tabs and stops forwarding", async () => {
    const conn = await connect();
    await ws.command(1, "chrome.debugger.attach", [{ tabId: 5 }, "1.3"]);
    calls.length = 0;
    let closed = false;
    conn.onclose = () => (closed = true);
    conn.close("bye");
    expect(closed).toBe(true);
    expect(ws.closed).toEqual([1000, "bye"]);
    expect(calls).toEqual([["debugger.detach", [{ tabId: 5 }]]]);
    expect(Object.values(events).every((e) => e.listeners.length === 0)).toBe(true);
  });

  test("the last group tab going away disconnects", async () => {
    const conn = await connect();
    events["tabs.onRemoved"].emit(5);
    events["tabs.onRemoved"].emit(6);
    expect(ws.closed?.[0]).toBe(1000);
    expect(conn.members.size).toBe(0);
  });
});
