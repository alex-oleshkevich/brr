import { SvelteSet } from 'svelte/reactivity';
import { api, unwrap } from './client';
import type { components } from './schema';

type Event = components['schemas']['Event'];

const STORAGE_KEY = 'brr.ui-client';
const UI_CLIENTS_LIMIT = 200;

// uiClientIds: every client that came from a web UI, so activity from any of them reads as "you".
export const session: { clientId: string | null; uiClientIds: SvelteSet<string> } = { clientId: null, uiClientIds: new SvelteSet() };

export function noteUiClient(e: Event): void {
  const data = e.data as { client_id?: string; kind?: string } | null;
  if (e.kind === 'client.created' && data?.kind === 'ui' && data.client_id) session.uiClientIds.add(data.client_id);
}

// Any call with the header tells whether the daemon still knows the client.
async function stillKnown(clientId: string): Promise<boolean> {
  const response = await fetch('/v1/system/info', { headers: { 'Brr-Client': clientId } }).catch(() => null);

  return response?.ok === true;
}

// Reloads and other tabs of this browser share one client, so a tab you took over stays yours; the daemon ends
// the client 2 min after the last tab's event stream closes.
export async function startSession(): Promise<void> {
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved && (await stillKnown(saved))) {
    session.clientId = saved;
  } else {
    const client = await unwrap(api.POST('/v1/clients', { body: { kind: 'ui', label: 'Web UI' } }));
    session.clientId = client.id;
    localStorage.setItem(STORAGE_KEY, client.id);
  }

  session.uiClientIds.add(session.clientId);
  void unwrap(api.GET('/v1/events', { params: { query: { kinds: 'client.created', order: 'desc', limit: UI_CLIENTS_LIMIT } } })).then(
    ({ events }) => events.forEach(noteUiClient),
    () => undefined,
  );
}
