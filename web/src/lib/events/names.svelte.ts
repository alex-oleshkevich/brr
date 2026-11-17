import { ApiError, api, unwrap } from '$lib/api/client';
import { events } from '$lib/api/events.svelte';

type Kind = 'instance' | 'profile' | 'tab' | 'client';

const MAX_NAMES = 500;
export const ATTACHED = 'Attached browser';

const fetchers: Record<Kind, (id: string) => Promise<string | null | undefined>> = {
  instance: async (id) => {
    const run = await unwrap(api.GET('/v1/instances/{instance_id}', { params: { path: { instance_id: id } } }));
    return run.kind === 'live' ? ATTACHED : run.profile_name;
  },
  profile: async (id) => (await unwrap(api.GET('/v1/profiles/{profile}', { params: { path: { profile: id } } }))).name,
  tab: async (id) => {
    const tab = await unwrap(api.GET('/v1/tabs/{tab_id}', { params: { path: { tab_id: id } } }));
    return tab.title || tab.url;
  },
  // There is no client lookup; the label is in the event that announced it.
  client: async (id) => {
    const { events } = await unwrap(api.GET('/v1/events', { params: { query: { subject: `client:${id}`, kinds: 'client.created', limit: 1 } } }));
    return (events[0]?.data as { label?: string } | undefined)?.label;
  },
};

// Ids in events resolve to names the first time they are shown; until then, and on a miss, a short id stands in.
class Names {
  private known = $state<Record<string, string>>({});
  private requests = new Map<string, Promise<void>>();

  constructor() {
    // A renamed profile must not keep its old name until the page reloads.
    events.on(['profile.updated'], (e) => this.forget(`profile:${(e.data as { profile_id?: string } | null)?.profile_id}`));
  }

  private forget(key: string): void {
    this.requests.delete(key);
    delete this.known[key];
  }

  private resolve(kind: Kind, id: string): Promise<void> {
    const key = `${kind}:${id}`;
    let request = this.requests.get(key);
    if (!request) {
      // Oldest first: a Map iterates in insertion order.
      if (this.requests.size >= MAX_NAMES) this.forget(this.requests.keys().next().value!);
      request = fetchers[kind](id).then(
        (name) => {
          if (name) this.known[key] = name;
        },
        // Only an unreachable daemon is worth asking again; a deleted item stays unknown.
        (e) => {
          if (e instanceof ApiError && e.offline) this.requests.delete(key);
        },
      );
      this.requests.set(key, request);
    }
    return request;
  }

  instance(id: string): string {
    void this.resolve('instance', id);
    return this.known[`instance:${id}`] ?? 'A browser';
  }

  profile(id: string): string {
    void this.resolve('profile', id);
    return this.known[`profile:${id}`] ?? 'A profile';
  }

  client(id: string): string | null {
    void this.resolve('client', id);
    return this.known[`client:${id}`] ?? null;
  }

  tab(id: string): string {
    void this.resolve('tab', id);
    return this.known[`tab:${id}`] ?? 'A tab';
  }

  // For callers that render once (toasts): wait until the names an event mentions are known.
  async preload(data: Record<string, unknown> | null): Promise<void> {
    const kinds: [Kind, string][] = [['instance', 'instance_id'], ['tab', 'tab_id'], ['profile', 'profile_id']];
    await Promise.all(
      kinds.flatMap(([kind, key]) => (typeof data?.[key] === 'string' ? [this.resolve(kind, data[key] as string)] : [])),
    );
  }
}

export const names = new Names();
