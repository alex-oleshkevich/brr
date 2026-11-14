// The "brr" tab group is the source of truth for which tabs a connection controls.
const GROUP_TITLE = "brr";
const GROUP_KEY = "group_id";
const GROUP_RETRIES = 5;
const GROUP_RETRY_MS = 100;

// chrome://, edge:// and devtools:// pages cannot be debugged.
export function isAttachable(tab: chrome.tabs.Tab): boolean {
  const url = tab.url || tab.pendingUrl || "";
  return !/^(chrome|edge|devtools):/.test(url);
}

// Chrome refuses to group while the user drags a tab; that passes within moments.
export async function groupTab(tabId: number, groupId?: number): Promise<number> {
  for (let attempt = 0; ; attempt++) {
    try {
      return await chrome.tabs.group({ tabIds: [tabId], groupId });
    } catch (error) {
      if (attempt >= GROUP_RETRIES || !String(error).includes("user may be dragging a tab")) throw error;
      await new Promise((resolve) => setTimeout(resolve, GROUP_RETRY_MS));
    }
  }
}

async function existingGroup(id: unknown): Promise<chrome.tabGroups.TabGroup | undefined> {
  if (typeof id !== "number") return undefined;
  try {
    return await chrome.tabGroups.get(id);
  } catch {
    return undefined;
  }
}

// Creates or reuses the brr group and adds the most recently active http(s) tab of the
// current window to it. Extension pages are skipped so popup.html opened as a tab still
// shares the page the user came from.
export async function shareActiveTab(): Promise<number> {
  const [tabs, stored] = await Promise.all([
    chrome.tabs.query({ currentWindow: true }),
    chrome.storage.session.get(GROUP_KEY),
  ]);
  const group = await existingGroup(stored[GROUP_KEY]);
  const candidate = tabs
    .filter((t) => t.id !== undefined && /^https?:/.test(t.url ?? ""))
    .sort((a, b) => (b.lastAccessed ?? 0) - (a.lastAccessed ?? 0))[0];
  if (!candidate) {
    if (!group) throw new Error("no http(s) tab in the current window to share");
    return group.id;
  }
  // Joining a group in another window would move the tab there.
  const reuse = group?.windowId === candidate.windowId ? group.id : undefined;
  const groupId = await groupTab(candidate.id!, reuse);
  await chrome.tabGroups.update(groupId, { title: GROUP_TITLE, color: "blue" });
  await chrome.storage.session.set({ [GROUP_KEY]: groupId });
  return groupId;
}
