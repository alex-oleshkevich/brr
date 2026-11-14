import { RelayConnection } from "./relayConnection";
import type { PopupReply, PopupRequest, Status } from "./protocol";
import { shareActiveTab } from "./tabGroup";

const RELAY_URL_KEY = "relay_url";

// The worker is suspended when idle and its globals are lost; a fresh worker is disconnected.
let connection: RelayConnection | undefined;
let connectedUrl = "";
let connecting = false;
void chrome.action.setBadgeText({ text: "" });

function openSocket(url: string): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    ws.onopen = () => resolve(ws);
    ws.onerror = () => reject(new Error(`cannot connect to ${url}`));
  });
}

async function connect(url: string): Promise<void> {
  if (connection || connecting) throw new Error("already connected");
  connecting = true;
  try {
    await open(url);
  } finally {
    connecting = false;
  }
}

async function open(url: string): Promise<void> {
  await chrome.storage.local.set({ [RELAY_URL_KEY]: url });
  const groupId = await shareActiveTab();
  const ws = await openSocket(url);
  const created = new RelayConnection(ws, groupId);
  connection = created;
  connectedUrl = url;
  created.onclose = () => {
    if (connection === created) connection = undefined;
    void chrome.action.setBadgeText({ text: "" });
  };
  try {
    await created.start();
  } catch (error) {
    created.close("start failed");
    throw error;
  }
  if (connection !== created) throw new Error("connection closed");
  void chrome.action.setBadgeText({ text: "ON" });
}

function status(): Status {
  if (!connection) return { connected: false, tabs: 0 };
  return {
    connected: true,
    url: connectedUrl,
    tabs: connection.members.size,
    notice: connection.blocked.size > 0 ? "brr can't control chrome:// pages" : undefined,
  };
}

async function handle(request: PopupRequest): Promise<PopupReply> {
  switch (request.type) {
    case "status":
      return status();
    case "connect":
      await connect(request.url);
      return { ok: true };
    case "disconnect":
      connection?.close("disconnected by user");
      return { ok: true };
  }
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  handle(message as PopupRequest).then(sendResponse, (error) =>
    sendResponse({ error: error instanceof Error ? error.message : String(error) }),
  );
  return true;
});
