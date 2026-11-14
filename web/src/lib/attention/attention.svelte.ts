import { SvelteSet } from 'svelte/reactivity';
import { api, unwrap } from '$lib/api/client';
import { liveQuery, type LiveQuery } from '$lib/api/live.svelte';
import type { components } from '$lib/api/schema';
import { outcome } from '$lib/instances/actions';

type HumanReason = components['schemas']['HumanReason'];
type Instance = components['schemas']['Instance'];
type Proxy = components['schemas']['Proxy'];

export type NeedsYou = {
  tabId: string;
  instanceId: string;
  agent: string;
  reason: HumanReason;
  url: string;
  deadline: number;
};

const DAY_MS = 86_400_000;
// A tab past its deadline has already timed out; the daemon moves it out of awaiting_human.
const DEADLINE_SLACK_MS = 60_000;
const EVENTS_LIMIT = 100;
const INSTANCES_LIMIT = 100;
const DISMISSED_KEY = 'brr.dismissed-attention';
const DISMISSED_MAX = 200;

export const REASON_VERBS: Record<HumanReason, string> = {
  login: 'log in',
  captcha: 'solve a captcha',
  dialog: 'answer a dialog',
};

export function hostOf(url: string): string {
  try {
    return new URL(url).hostname || url;
  } catch {
    return url;
  }
}

async function loadNeedsYou(): Promise<NeedsYou[]> {
  const { events } = await unwrap(
    api.GET('/v1/events', { params: { query: { kinds: 'tab.awaiting_human', from_ts: Date.now() - DAY_MS, order: 'desc', limit: EVENTS_LIMIT } } }),
  );
  // The newest mark per tab wins; the Tab record then says whether it still waits.
  const marks = new Map<string, { instanceId: string; reason: HumanReason; deadline: number }>();
  for (const e of events) {
    const data = e.data as { tab_id: string; instance_id: string; reason: HumanReason; deadline: number };
    if (!marks.has(data.tab_id) && data.deadline > Date.now() - DEADLINE_SLACK_MS) {
      marks.set(data.tab_id, { instanceId: data.instance_id, reason: data.reason, deadline: data.deadline });
    }
  }

  const tabs = await Promise.all(
    [...marks].map(async ([tabId, mark]) => ({
      mark,
      tab: await unwrap(api.GET('/v1/tabs/{tab_id}', { params: { path: { tab_id: tabId } } })).catch(() => null),
    })),
  );
  return tabs
    .filter(({ tab }) => tab?.state === 'awaiting_human')
    .map(({ mark, tab }) => ({
      tabId: tab!.id,
      instanceId: mark.instanceId,
      agent: tab!.lease && tab!.lease.client_kind !== 'ui' ? tab!.lease.client_label : 'An agent',
      reason: mark.reason,
      url: tab!.url,
      deadline: mark.deadline,
    }))
    .toSorted((a, b) => a.deadline - b.deadline);
}

class Attention {
  private needsQuery: LiveQuery<NeedsYou[]> | undefined;
  private failedQuery: LiveQuery<Instance[]> | undefined;
  private proxyQuery: LiveQuery<Proxy[]> | undefined;

  // Items a person cleared from the Dashboard; kept in this browser, so they stay gone after a reload.
  readonly dismissed = new SvelteSet<string>();

  // Called once from the layout once the session is ready; every page and the global bar share these queries.
  init(): void {
    JSON.parse(localStorage.getItem(DISMISSED_KEY) ?? '[]').forEach((key: string) => this.dismissed.add(key));
    this.needsQuery = liveQuery(loadNeedsYou, {
      kinds: ['tab.awaiting_human', 'tab.state_changed', 'tab.closed', 'instance.state_changed'],
      // Tab state changes are frequent; only the awaiting_human edge matters.
      match: (e) => {
        if (e.kind !== 'tab.state_changed') return true;
        const data = e.data as { from?: string; to?: string };
        return data.from === 'awaiting_human' || data.to === 'awaiting_human';
      },
    });
    this.failedQuery = liveQuery(
      async () => {
        const since = Date.now() - DAY_MS;
        // A crashed browser usually reads "stopped" by the time it is listed; its end reason says why.
        const { instances } = await unwrap(api.GET('/v1/instances', { params: { query: { state: 'crashed,failed,stopped', limit: INSTANCES_LIMIT } } }));
        return instances
          .filter((i) => outcome(i)?.tone === 'danger' && (i.ended_at ?? i.last_activity_at) >= since).toSorted((a, b) => (b.ended_at ?? b.last_activity_at) - (a.ended_at ?? a.last_activity_at));
      },
      { kinds: ['instance.state_changed'] },
    );
    this.proxyQuery = liveQuery(async () => (await unwrap(api.GET('/v1/proxies'))).proxies, { kinds: ['proxy.health'] });
  }

  get needsYou(): NeedsYou[] {
    return this.needsQuery?.data ?? [];
  }

  // False until the first load, so nothing is reported resolved before it is known.
  get needsYouLoaded(): boolean {
    return this.needsQuery?.data !== undefined;
  }

  get failures(): Instance[] {
    return this.failedQuery?.data ?? [];
  }

  dismiss(key: string): void {
    this.dismissed.add(key);
    localStorage.setItem(DISMISSED_KEY, JSON.stringify([...this.dismissed].slice(-DISMISSED_MAX)));
  }

  get unhealthyProxies(): Proxy[] {
    return (this.proxyQuery?.data ?? []).filter((p) => p.healthy === false);
  }

  get loading(): boolean {
    return !this.needsQuery || this.needsQuery.loading || this.failedQuery!.loading || this.proxyQuery!.loading;
  }

  get error() {
    return this.needsQuery?.error ?? this.failedQuery?.error ?? this.proxyQuery?.error ?? null;
  }

  reload(): void {
    this.needsQuery?.reload();
    this.failedQuery?.reload();
    this.proxyQuery?.reload();
  }
}

export const attention = new Attention();
