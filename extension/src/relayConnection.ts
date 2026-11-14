/**
 * Adapted from Playwright's packages/extension/src/relayConnection.ts.
 *
 * Copyright (c) Microsoft Corporation.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
import { ALLOWED_COMMANDS, PROTOCOL_VERSION, type Command, type Message } from "./protocol";
import { groupTab, isAttachable } from "./tabGroup";

export interface Socket {
  send(data: string): void;
  close(code?: number, reason?: string): void;
  onmessage: ((event: { data: unknown }) => void) | null;
  onclose: (() => void) | null;
}

type Listenable<F extends (...args: any[]) => unknown> = chrome.Event<F>;

const noop = () => {};
const PING_MS = 20_000;

export class RelayConnection {
  // Tabs in the brr group that the relay may see and control.
  readonly members = new Set<number>();
  // Tabs in the group that cannot be debugged (chrome://, devtools://).
  readonly blocked = new Set<number>();
  private attached = new Set<number>();
  private removers: Array<() => void> = [];
  private closed = false;
  private ping?: ReturnType<typeof setInterval>;

  onclose?: () => void;

  constructor(
    private ws: Socket,
    private groupId: number,
  ) {
    ws.onmessage = (event) => void this.onMessage(String(event.data));
    ws.onclose = () => this.shutdown();
  }

  // hello, one chrome.tabs.onCreated per group tab, then initialized: the relay holds
  // browser-level CDP until it sees initialized.
  async start(): Promise<void> {
    const tabs = await chrome.tabs.query({ groupId: this.groupId });
    if (this.closed) return;
    this.send({
      method: "extension.hello",
      params: [
        {
          protocol: PROTOCOL_VERSION,
          version: chrome.runtime.getManifest().version,
          user_agent: navigator.userAgent,
        },
      ],
    });
    for (const tab of tabs) this.sync(tab);
    this.listen();
    this.ping = setInterval(() => this.send({ method: "extension.ping" }), PING_MS);
    this.send({ method: "extension.initialized", params: [] });
  }

  close(reason: string): void {
    this.ws.close(1000, reason);
    // onclose fires asynchronously; shut down now so no event reaches a closed socket.
    this.shutdown();
  }

  private shutdown(): void {
    if (this.closed) return;
    this.closed = true;
    clearInterval(this.ping);
    for (const remove of this.removers) remove();
    this.removers = [];
    for (const tabId of this.attached) this.detach(tabId);
    this.attached.clear();
    this.onclose?.();
  }

  private send(message: Message): void {
    if (!this.closed) this.ws.send(JSON.stringify(message));
  }

  private detach(tabId: number): void {
    chrome.debugger.detach({ tabId }).catch(noop);
  }

  private on<F extends (...args: any[]) => unknown>(event: Listenable<F>, listener: F): void {
    event.addListener(listener);
    this.removers.push(() => event.removeListener(listener));
  }

  // Events are filtered to member tabs: forwarding anything else would expose the user's other tabs.
  private listen(): void {
    this.on(chrome.tabs.onCreated, (tab) => void this.sync(tab));
    this.on(chrome.tabs.onUpdated, (tabId, changeInfo, tab) => {
      if (this.sync(tab) || !this.members.has(tabId)) return;
      if (changeInfo.url !== undefined || changeInfo.title !== undefined)
        this.send({ method: "chrome.tabs.onUpdated", params: [tabId, changeInfo, tab] });
    });
    this.on(chrome.tabs.onRemoved, (tabId) => {
      this.blocked.delete(tabId);
      if (!this.members.delete(tabId)) return;
      this.attached.delete(tabId);
      this.send({ method: "chrome.debugger.onDetach", params: [{ tabId }, "target_closed"] });
      this.send({ method: "chrome.tabs.onRemoved", params: [tabId] });
      this.closeIfEmpty();
    });
    this.on(chrome.debugger.onEvent, (source, method, params) => {
      if (source.tabId !== undefined && this.members.has(source.tabId))
        this.send({ method: "chrome.debugger.onEvent", params: [source, method, params] });
    });
    this.on(chrome.debugger.onDetach, (source, reason) => {
      if (source.tabId === undefined || !this.members.has(source.tabId)) return;
      this.attached.delete(source.tabId);
      this.send({ method: "chrome.debugger.onDetach", params: [source, reason] });
    });
  }

  // Reconciles one tab with the group. Returns true when its membership changed.
  private sync(tab: chrome.tabs.Tab): boolean {
    const id = tab.id;
    if (id === undefined) return false;
    if (tab.groupId !== this.groupId) {
      this.blocked.delete(id);
      return this.release(id);
    }
    if (!isAttachable(tab)) {
      this.blocked.add(id);
      return this.release(id);
    }
    this.blocked.delete(id);
    if (this.members.has(id)) return false;
    this.members.add(id);
    this.send({ method: "chrome.tabs.onCreated", params: [tab] });
    return true;
  }

  // The tab left the group (or became undebuggable) but still exists.
  private release(tabId: number): boolean {
    if (!this.members.delete(tabId)) return false;
    this.attached.delete(tabId);
    this.detach(tabId);
    this.send({ method: "chrome.debugger.onDetach", params: [{ tabId }, "target_closed"] });
    this.send({ method: "chrome.tabs.onRemoved", params: [tabId] });
    this.closeIfEmpty();
    return true;
  }

  private closeIfEmpty(): void {
    if (this.members.size === 0) this.close("no tabs left in the brr group");
  }

  private async onMessage(text: string): Promise<void> {
    let command: Command;
    try {
      command = JSON.parse(text);
    } catch {
      return; // no id to answer
    }
    let reply: Message;
    try {
      reply = { id: command.id, result: (await this.run(command)) ?? null };
    } catch (error) {
      reply = { id: command.id, error: error instanceof Error ? error.message : String(error) };
    }
    this.send(reply);
  }

  private async run({ method, params = [] }: Command): Promise<unknown> {
    if (!ALLOWED_COMMANDS.has(method)) throw new Error("command not allowed");
    const [, namespace, fn] = method.split(".");
    const creating = method === "chrome.tabs.create";
    const first = params[0] as number | { tabId?: number } | undefined;
    const tabId = typeof first === "number" ? first : first?.tabId;
    if (!creating && (tabId === undefined || !this.members.has(tabId)))
      throw new Error(`tab ${tabId} is not in the brr tab group`);

    const api = (chrome as unknown as Record<string, Record<string, (...args: unknown[]) => Promise<unknown>>>)[
      namespace
    ];
    const result = await api[fn](...params);

    if (method === "chrome.debugger.attach") {
      // chrome.debugger.detach does not fire onDetach for its caller, so only attach is tracked.
      // Also when the tab left the group while the attach was in flight.
      if (this.closed || !this.members.has(tabId!)) this.detach(tabId!);
      else this.attached.add(tabId!);
    } else if (method === "chrome.debugger.detach") {
      this.attached.delete(tabId!);
    } else if (creating) {
      const tab = result as chrome.tabs.Tab;
      try {
        await groupTab(tab.id!, this.groupId);
      } catch (error) {
        await chrome.tabs.remove(tab.id!).catch(noop);
        throw error;
      }
      this.sync({ ...tab, groupId: this.groupId });
    }
    return result;
  }
}
