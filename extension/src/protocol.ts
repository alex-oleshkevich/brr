export const PROTOCOL_VERSION = 1;
export const DEFAULT_RELAY_URL = "ws://127.0.0.1:9898/v1/extension-relay";

// chrome.* commands the relay may invoke; params are positional and spread into the call.
export const ALLOWED_COMMANDS = new Set([
  "chrome.debugger.attach",
  "chrome.debugger.detach",
  "chrome.debugger.sendCommand",
  "chrome.tabs.create",
  "chrome.tabs.remove",
  "chrome.tabs.update",
]);

// extension.ping (no params) is sent every 20 s while connected so the MV3 service worker is
// not suspended; the relay ignores it.
export interface Command {
  id: number;
  method: string;
  params?: unknown[];
}

export type Message =
  | { id: number; result: unknown }
  | { id: number; error: string }
  | { method: string; params?: unknown[] };

// popup -> background
export type PopupRequest =
  | { type: "status" }
  | { type: "connect"; url: string }
  | { type: "disconnect" };

export interface Status {
  connected: boolean;
  url?: string;
  tabs: number;
  notice?: string;
}

export type PopupReply = Status | { ok: true } | { error: string };
