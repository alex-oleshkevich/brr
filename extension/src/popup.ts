import { DEFAULT_RELAY_URL, type PopupReply, type PopupRequest, type Status } from "./protocol";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

async function request(message: PopupRequest): Promise<PopupReply> {
  return (await chrome.runtime.sendMessage(message)) as PopupReply;
}

async function initialUrl(): Promise<string> {
  try {
    const response = await fetch(chrome.runtime.getURL("brr.json"));
    if (response.ok) return ((await response.json()) as { relay_url: string }).relay_url;
  } catch {
    // no brr.json: not a daemon-materialized build
  }
  const stored = (await chrome.storage.local.get("relay_url")).relay_url;
  return typeof stored === "string" ? stored : DEFAULT_RELAY_URL;
}

function render(status: Status): void {
  $("idle").hidden = status.connected;
  $("connected").hidden = !status.connected;
  $("connected-info").textContent = `Connected to ${status.url}. ${status.tabs} shared tab(s).`;
  $("notice").textContent = status.notice ?? "";
}

function showError(reply: PopupReply): boolean {
  $("error").textContent = "error" in reply ? reply.error : "";
  return "error" in reply;
}

async function main(): Promise<void> {
  const input = $<HTMLInputElement>("url");
  const consent = (): void => {
    $("consent").textContent =
      `Allow brr at ${input.value} to control the tabs in the brr tab group? ` +
      "Tabs show a 'being debugged' bar while connected.";
  };
  input.value = await initialUrl();
  input.addEventListener("input", consent);
  consent();

  $("allow").addEventListener("click", async () => {
    if (showError(await request({ type: "connect", url: input.value }))) return;
    render((await request({ type: "status" })) as Status);
  });
  $("cancel").addEventListener("click", () => window.close());
  $("disconnect").addEventListener("click", async () => {
    await request({ type: "disconnect" });
    render((await request({ type: "status" })) as Status);
  });

  render((await request({ type: "status" })) as Status);
}

void main();
