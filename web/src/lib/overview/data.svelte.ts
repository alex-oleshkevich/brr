import { api, unwrap } from '$lib/api/client';
import { liveQuery, type LiveQuery } from '$lib/api/live.svelte';
import type { components } from '$lib/api/schema';

type Schemas = components['schemas'];

const DAY_MS = 86_400_000;
const BACKUP_EVENTS_LIMIT = 200;
// Failed launches never set started_at; look past a few of them for a run that did start.
const FIRST_RUN_LIMIT = 100;
const PAGE_LIMIT = 200;
const AGENT_KINDS = ['mcp', 'rest'];

// Queries shared by several Overview cards; call once from the page so the effects belong to it.
export interface OverviewData {
  builds: LiveQuery<Schemas['BuildView'][]>;
  profiles: LiveQuery<Schemas['Profile'][]>;
  templates: LiveQuery<Schemas['Template'][]>;
  pools: LiveQuery<Schemas['Pool'][]>;
  captcha: LiveQuery<Schemas['SettingsView']>;
  stats: LiveQuery<Schemas['DayStats'][]>;
  traffic: LiveQuery<Schemas['UsageHistoryOutput']>;
  balance: LiveQuery<Schemas['CaptchaBalanceOutput'] | null>;
  storage: LiveQuery<Schemas['StorageUsageOutput']>;
  failedBackups: LiveQuery<{ profileId: string; error: string }[]>;
  clients: LiveQuery<Schemas['ClientView'][]>;
  firstRun: LiveQuery<boolean>;
  hasAgent: () => boolean;
}

export function overviewData(): OverviewData {
  const builds = liveQuery(async () => (await unwrap(api.GET('/v1/builds'))).builds, {
    kinds: ['build.*'],
  });
  const profiles = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, {
    kinds: ['profile.*', 'login.*'],
  });
  const templates = liveQuery(async () => (await unwrap(api.GET('/v1/templates'))).templates, {
    kinds: ['template.*'],
  });
  const pools = liveQuery(
    async () => {
      const all: Schemas['Pool'][] = [];
      let cursor: string | undefined;
      do {
        const page = await unwrap(api.GET('/v1/proxy-pools', { params: { query: { limit: PAGE_LIMIT, cursor } } }));
        all.push(...page.pools);
        cursor = page.next_cursor ?? undefined;
      } while (cursor);

      return all;
    },
    { kinds: ['proxy.*'] },
  );
  const captcha = liveQuery(() => unwrap(api.GET('/v1/captcha/settings')), { kinds: ['captcha.*'] });
  const stats = liveQuery(async () => (await unwrap(api.GET('/v1/captcha/stats', { params: { query: { days: 7 } } }))).days, {
    kinds: ['captcha.*'],
  });
  // A running run's bytes are in the current bucket, so the totals follow every state change.
  const traffic = liveQuery(() => unwrap(api.GET('/v1/usage/history', { params: { query: { window: '7d' } } })), {
    kinds: ['instance.*'],
  });
  // The balance is a paid provider call: fetched on load and on captcha events only, and only with a key.
  const balance = liveQuery(
    async () => {
      if (!captcha.data?.api_key?.set) return null;

      return unwrap(api.GET('/v1/captcha/balance'));
    },
    { kinds: ['captcha.*'] },
  );
  const storage = liveQuery(() => unwrap(api.GET('/v1/storage/usage')), {
    kinds: ['backup.*', 'instance.state_changed', 'retention.*', 'artifact.*'],
  });
  // The newest backup event per Profile decides whether its last backup failed.
  const failedBackups = liveQuery(
    async () => {
      const { events } = await unwrap(
        api.GET('/v1/events', {
          params: { query: { kinds: 'backup.failed,backup.completed', from_ts: Date.now() - 7 * DAY_MS, order: 'desc', limit: BACKUP_EVENTS_LIMIT } },
        }),
      );
      const newest = new Map<string, { failed: boolean; error?: string }>();
      for (const e of events) {
        const data = e.data as { profile_id: string; error?: string };
        if (!newest.has(data.profile_id)) newest.set(data.profile_id, { failed: e.kind === 'backup.failed', error: data.error });
      }

      return [...newest].filter(([, v]) => v.failed).map(([profileId, v]) => ({ profileId, error: v.error ?? 'The last backup failed' }));
    },
    { kinds: ['backup.*'] },
  );
  const clients = liveQuery(async () => (await unwrap(api.GET('/v1/clients'))).clients, { kinds: ['client.*', 'lease.*'] });
  const firstRun = liveQuery(async () => (await unwrap(api.GET('/v1/instances', { params: { query: { limit: FIRST_RUN_LIMIT } } }))).instances.some((i) => i.started_at), {
    kinds: ['instance.*'],
  });

  return {
    builds,
    profiles,
    templates,
    pools,
    captcha,
    stats,
    traffic,
    balance,
    storage,
    failedBackups,
    clients,
    firstRun,
    hasAgent: () => clients.data?.some((c) => AGENT_KINDS.includes(c.kind)) ?? false,
  };
}

// The 7-day history comes in 6-hour buckets; the Overview shows bytes per local day.
export function trafficByDay(history: Schemas['UsageHistoryOutput'] | undefined): { start: number; bytes: number }[] {
  const days = new Map<number, number>();
  for (const bucket of history?.buckets ?? []) {
    const day = new Date(bucket.start_ms);
    day.setHours(0, 0, 0, 0);
    days.set(day.getTime(), (days.get(day.getTime()) ?? 0) + bucket.traffic_bytes);
  }

  return [...days].map(([start, bytes]) => ({ start, bytes }));
}
