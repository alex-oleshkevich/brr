// Only the chrome.* members this extension uses.
declare namespace chrome {
  interface Event<F extends (...args: any[]) => unknown> {
    addListener(callback: F): void;
    removeListener(callback: F): void;
  }

  namespace tabs {
    interface Tab {
      id?: number;
      url?: string;
      pendingUrl?: string;
      title?: string;
      groupId: number;
      windowId: number;
      lastAccessed?: number;
    }
    interface ChangeInfo {
      url?: string;
      title?: string;
    }
    function query(info: { currentWindow?: boolean; groupId?: number }): Promise<Tab[]>;
    function remove(tabId: number): Promise<void>;
    function group(options: { tabIds: number[]; groupId?: number }): Promise<number>;
    const onCreated: Event<(tab: Tab) => void>;
    const onUpdated: Event<(tabId: number, changeInfo: ChangeInfo, tab: Tab) => void>;
    const onRemoved: Event<(tabId: number) => void>;
  }

  namespace tabGroups {
    interface TabGroup {
      id: number;
      windowId: number;
    }
    function get(groupId: number): Promise<TabGroup>;
    function update(groupId: number, props: { title: string; color: "blue" }): Promise<TabGroup>;
  }

  namespace debugger {
    interface Debuggee {
      tabId?: number;
    }
    function detach(target: Debuggee): Promise<void>;
    const onEvent: Event<(source: Debuggee, method: string, params?: unknown) => void>;
    const onDetach: Event<(source: Debuggee, reason: string) => void>;
  }

  namespace storage {
    interface Area {
      get(key: string): Promise<Record<string, unknown>>;
      set(items: Record<string, unknown>): Promise<void>;
    }
    const local: Area;
    const session: Area;
  }

  namespace runtime {
    function getManifest(): { version: string };
    function getURL(path: string): string;
    function sendMessage(message: unknown): Promise<unknown>;
    const onMessage: Event<
      (message: unknown, sender: unknown, sendResponse: (response: unknown) => void) => boolean | void
    >;
  }

  namespace action {
    function setBadgeText(details: { text: string }): Promise<void>;
  }
}
