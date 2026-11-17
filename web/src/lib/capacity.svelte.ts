import { api, unwrap, ApiError } from './api/client';
import { events } from './api/events.svelte';
import { liveQuery, type LiveQuery } from './api/live.svelte';
import { fetchAll } from './api/paged.svelte';
import type { components } from './api/schema';
import { clock } from './clock.svelte';
import { ACTIVE_STATES } from './state';

type Info = components['schemas']['SystemInfo'];
type Limits = components['schemas']['LimitsView'];
type QueueEntry = components['schemas']['QueueEntry'];
type Instance = components['schemas']['Instance'];
type Lease = components['schemas']['Lease'];
type Usage = { instances: Instance[]; leases: Lease[] };

const REFRESH_MS = 5000;
const PRESSURE_WINDOW_MS = 60_000;
const PAGE = 500;

// The top-bar pill: `full` means a count limit is reached, `pressure` means memory or load.
export type Pressure = { state: 'ok' | 'full' | 'pressure'; text: string };

class Capacity {
  private infoQuery: LiveQuery<Info> | undefined;
  private limitsQuery: LiveQuery<Limits> | undefined;
  private queueQuery: LiveQuery<QueueEntry[]> | undefined;
  private usageQuery: LiveQuery<Usage> | undefined;
  private lastPressureAt = $state(0);

  // Called once from the layout: the three polls are shared by the topbar and the Dashboard.
  init(): void {
    const instanceKinds = ['instance.state_changed', 'instance.queued', 'admission.decided'];
    // Only system info changes without an event, so it alone is polled.
    this.infoQuery = liveQuery(() => unwrap(api.GET('/v1/system/info')), {
      kinds: [...instanceKinds, 'system.memory_pressure'],
      refreshMs: REFRESH_MS,
    });
    this.limitsQuery = liveQuery(() => unwrap(api.GET('/v1/limits')), { kinds: ['limits.changed', 'settings.changed'] });
    this.queueQuery = liveQuery(async () => (await unwrap(api.GET('/v1/admission/queue'))).queue, { kinds: instanceKinds });

    this.usageQuery = liveQuery(
      async () => {
        const [instances, leases] = await Promise.all([
          fetchAll(async (cursor) => {
            const page = await unwrap(api.GET('/v1/instances', { params: { query: { state: ACTIVE_STATES.join(','), limit: PAGE, cursor } } }));
            return { items: page.instances, next: page.next_cursor };
          }),
          fetchAll(async (cursor) => {
            const page = await unwrap(api.GET('/v1/leases', { params: { query: { limit: PAGE, cursor } } }));
            return { items: page.leases, next: page.next_cursor };
          }),
        ]);
        return { instances, leases };
      },
      { kinds: ['instance.*', 'tab.opened', 'tab.closed', 'lease.*'] },
    );

    $effect(() => events.on(['system.memory_pressure'], (e) => (this.lastPressureAt = e.ts)));
  }

  get info() {
    return this.infoQuery?.data;
  }

  // Display modes that can't start on this host, mapped to the reason.
  get displayUnavailable(): Map<string, string> {
    return new Map(this.info?.display_unavailable.map((u) => [u.display, u.reason] as const));
  }

  get limits() {
    return this.limitsQuery?.data;
  }

  get queue() {
    return this.queueQuery?.data;
  }

  get usage() {
    return this.usageQuery?.data;
  }

  get browsers(): number {
    return this.info?.browsers_running ?? 0;
  }

  // Browsers running on Hosts and the Hosts' combined limit; 0 when there are no Hosts.
  get hostBrowsers(): number {
    return this.info?.hosts_running ?? 0;
  }

  get hostMax(): number {
    return this.info?.hosts_max ?? 0;
  }

  // "1/1 · Hosts 2/2" detail behind the pill's total.
  get breakdown(): string | null {
    const { limits } = this;
    if (!limits || this.hostMax === 0) return null;
    return `This machine ${this.browsers}/${limits.max_instances} · Hosts ${this.hostBrowsers}/${this.hostMax}`;
  }

  get tabs(): number {
    return this.usage?.instances.reduce((n, i) => n + i.tab_count, 0) ?? 0;
  }

  get loading() {
    return !this.infoQuery || this.infoQuery.loading || this.limitsQuery!.loading || this.queueQuery!.loading || this.usageQuery!.loading;
  }

  get error(): ApiError | null {
    return this.infoQuery?.error ?? this.limitsQuery?.error ?? this.queueQuery?.error ?? this.usageQuery?.error ?? null;
  }

  reload(): void {
    this.infoQuery?.reload();
    this.limitsQuery?.reload();
    this.queueQuery?.reload();
    this.usageQuery?.reload();
  }

  get pressure(): Pressure {
    const { info, limits } = this;
    // Admission and the reaper act on free RAM and load only in adaptive mode.
    const adaptive = limits?.mode === 'adaptive';
    const memory = (adaptive && info && info.mem_available_mb < limits.adaptive.min_free_ram_mb) || clock.now - this.lastPressureAt < PRESSURE_WINDOW_MS;

    if (memory) return { state: 'pressure', text: 'Low memory · new browsers wait' };
    if (adaptive && info && info.load_per_core > limits.adaptive.max_load_per_core) return { state: 'pressure', text: 'High load · new browsers wait' };

    const { tabs } = this;
    const browsers = this.browsers + this.hostBrowsers;
    const max = (limits?.max_instances ?? 0) + this.hostMax;
    const waiting = this.queue?.length ?? 0;
    if (limits && info?.at_capacity) {
      return { state: 'full', text: `At capacity · ${browsers}/${max} browsers${waiting > 0 ? ` · ${waiting} waiting` : ''}` };
    }
    if (limits && waiting > 0) return { state: 'full', text: `${waiting} waiting to start · ${browsers}/${max} browsers` };
    if (limits && tabs >= limits.max_tabs_total) return { state: 'full', text: `At capacity · ${tabs}/${limits.max_tabs_total} tabs` };

    return { state: 'ok', text: limits ? `${browsers}/${max} browsers` : 'Capacity OK' };
  }
}

export const capacity = new Capacity();
