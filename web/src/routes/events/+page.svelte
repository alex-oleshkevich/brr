<script lang="ts">
  import PagedFooter from '$lib/ui/PagedFooter.svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { api, unwrap } from '$lib/api/client';
  import { events, type BrrEvent } from '$lib/api/events.svelte';
  import { liveQuery } from '$lib/api/live.svelte';
  import { pagedQuery } from '$lib/api/paged.svelte';
  import { clock } from '$lib/clock.svelte';
  import { names } from '$lib/events/names.svelte';
  import { CATEGORIES, KINDS, kindLabel, persistedKinds, type Category } from '$lib/events/kinds';
  import { summarize } from '$lib/events/summary';
  import { absolute, relative } from '$lib/format';
  import { saveBlob } from '$lib/download';
  import { stateLabel } from '$lib/state';
  import { profileLabel } from '$lib/instances/actions';
  import Button from '$lib/ui/Button.svelte';
  import Checkbox from '$lib/ui/Checkbox.svelte';
  import CodeBlock from '$lib/ui/CodeBlock.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  const LIMIT = 100;
  const MAX_ROWS = 1000;
  const FADE_MS = 1200;
  const EXPORT_PAGE = 500;
  const EXPORT_MAX = 10_000;

  const TIMES: Record<string, number> = { '15m': 15 * 60_000, '1h': 3_600_000, '24h': 86_400_000, '7d': 604_800_000 };
  const timeOptions = [
    { value: '15m', label: 'Last 15 minutes' },
    { value: '1h', label: 'Last hour' },
    { value: '24h', label: 'Last 24 hours' },
    { value: '7d', label: 'Last 7 days' },
    { value: 'all', label: 'All time' },
  ];
  const categoryOptions = [{ value: '', label: 'All activity' }, ...CATEGORIES.map((c) => ({ value: c.id, label: c.label }))];
  const SUBJECT_NAMES: Record<string, string> = {
    instance: 'Browser',
    tab: 'Tab',
    client: 'Agent',
    lease: 'Control',
    build: 'Browser version',
    profile: 'Profile',
    extension: 'Extension',
    artifact: 'File',
    proxy: 'Proxy',
    system: 'System',
  };
  const PICKER_BROWSERS = 50;

  const initial = page.url.searchParams;
  let category = $state(initial.get('category') ?? '');
  // What the list is about, as "type:id": a browser or profile from the picker, or any subject clicked in the table.
  let focus = $state(initial.get('subject') ?? (initial.get('instance') ? `instance:${initial.get('instance')}` : ''));
  let time = $state(initial.get('time') ?? 'all');
  // Every UI session creates and ends a client; those rows bury the rest, so they are hidden unless asked for.
  const CLIENT_NOISE = ['client.created', 'client.ended'];
  let showClients = $state(initial.get('clients') === '1');
  const [focusType, focusId] = $derived(focus.split(/:(.*)/s));
  const instance = $derived(focusType === 'instance' ? focusId : null);
  // A browser matches on every event that mentions it (tabs, leases); anything else matches on the event's own subject.
  const subject = $derived(focus && focusType !== 'instance' ? focus : '');

  const profiles = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, { kinds: ['profile.created', 'profile.updated', 'profile.deleted'] });
  const browsers = liveQuery(
    async () => (await unwrap(api.GET('/v1/instances', { params: { query: { limit: PICKER_BROWSERS } } }))).instances,
    { kinds: ['instance.state_changed'] },
  );

  const focusOptions = $derived.by(() => {
    const builtinIds = new Set((profiles.data ?? []).filter((p) => p.builtin).map((p) => p.id));
    const browserOptions = (browsers.data ?? [])
      .filter((i) => !(i.profile_id && builtinIds.has(i.profile_id)))
      .toSorted((a, b) => b.queued_at - a.queued_at)
      .map((i) => ({ value: `instance:${i.id}`, label: `${profileLabel(i)} · ${stateLabel(i.state).toLowerCase()} ${relative(i.started_at ?? i.queued_at, clock.now)}` }));
    const profileOptions = (profiles.data ?? []).filter((p) => !p.builtin).map((p) => ({ value: `profile:${p.id}`, label: p.name }));
    const known = new Set([...browserOptions, ...profileOptions].map((o) => o.value));
    return [
      { value: '', label: 'Everything' },
      ...(focus && !known.has(focus) ? [{ value: focus, label: subjectLabel(focusType, focusId) }] : []),
      { group: 'Browsers', options: browserOptions },
      { group: 'Profiles', options: profileOptions },
    ];
  });

  // Only things with a name get one; an id fragment says nothing to a person.
  function focusLabel(type: string, id: string): string | null {
    if (type === 'instance') return names.instance(id);
    if (type === 'profile') return names.profile(id);
    if (type === 'tab') return names.tab(id);
    return null;
  }

  function subjectLabel(type: string, id: string): string {
    const label = SUBJECT_NAMES[type] ?? type;
    const name = focusLabel(type, id);

    return name ? `${label} · ${name}` : label;
  }

  // The URL mirrors the filters so a link can pre-filter the page.
  function updateUrl(changes: Record<string, string | null>) {
    const url = new URL(page.url);
    for (const [key, value] of Object.entries(changes)) {
      if (value) url.searchParams.set(key, value);
      else url.searchParams.delete(key);
    }
    if (url.search !== page.url.search) void goto(url, { replaceState: true, keepFocus: true, noScroll: true });
  }

  $effect(() => {
    updateUrl({ category: category || null, subject: focus || null, instance: null, time: time === 'all' ? null : time, clients: showClients ? '1' : null });
  });

  const kinds = $derived(
    category
      ? persistedKinds(category as Category)
      : showClients
        ? []
        : KINDS.filter((k) => !k.transient && !CLIENT_NOISE.includes(k.kind)).map((k) => k.kind),
  );
  const filtered = $derived(Boolean(category || focus || time !== 'all'));
  // Fixed per filter change so every page and the export see the same window.
  const fromTs = $derived(time === 'all' ? undefined : Date.now() - TIMES[time]);

  function query(extra: { limit: number; cursor?: string }) {
    return {
      kinds: kinds.length ? kinds.join(',') : undefined,
      subject: subject || undefined,
      instance_id: instance ?? undefined,
      from_ts: fromTs,
      order: 'desc' as const,
      ...extra,
    };
  }

  const list = pagedQuery(
    async (cursor) => {
      const page = await unwrap(api.GET('/v1/events', { params: { query: query({ limit: LIMIT, cursor }) } }));
      return { items: page.events, next: page.next_cursor };
    },
    { kinds: [], key: (e) => e.seq, moreError: "Couldn't load more events", max: MAX_ROWS },
  );

  let fresh = $state(new Set<number>());
  let open = $state(new Set<number>());
  let exporting = $state(false);

  function matches(e: BrrEvent): boolean {
    if (KINDS.find((k) => k.kind === e.kind)?.transient !== false) return false;
    if (kinds.length && !kinds.includes(e.kind)) return false;
    if (subject && `${e.subject.type}:${e.subject.id}` !== subject) return false;
    if (instance && (e.data as { instance_id?: string } | null)?.instance_id !== instance) return false;
    return true;
  }

  $effect(() => {
    return events.on('*', (e) => {
      if (!matches(e) || list.items?.some((row) => row.seq === e.seq)) return;
      list.prepend(e);
      fresh = new Set(fresh).add(e.seq);
      setTimeout(() => {
        const next = new Set(fresh);
        next.delete(e.seq);
        fresh = next;
      }, FADE_MS);
    });
  });

  const pad = (n: number) => String(n).padStart(2, '0');

  async function exportJson() {
    exporting = true;
    try {
      const all: BrrEvent[] = [];
      let cursor: string | undefined;
      while (all.length < EXPORT_MAX) {
        const next = await unwrap(api.GET('/v1/events', { params: { query: query({ limit: EXPORT_PAGE, cursor }) } }));
        all.push(...next.events);
        if (!next.next_cursor) break;
        cursor = next.next_cursor;
      }

      const d = new Date();
      const stamp = `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
      saveBlob(new Blob([JSON.stringify(all.slice(0, EXPORT_MAX), null, 2)]), `brr-events-${stamp}.json`);
    } catch (e) {
      toast.apiError(e, "Couldn't export activity");
    } finally {
      exporting = false;
    }
  }

  function toggle(seq: number) {
    const next = new Set(open);
    if (!next.delete(seq)) next.add(seq);
    open = next;
  }

  function clear() {
    category = '';
    focus = '';
    time = 'all';
    showClients = false;
    updateUrl({ category: null, subject: null, time: null, instance: null, clients: null });
  }

  const columns = [
    { key: 'time', label: 'Time', width: '140px' },
    { key: 'what', label: 'What happened', width: '22%' },
    { key: 'subject', label: 'About', width: '20%' },
    { key: 'summary', label: 'Details' },
  ];

  const empty = $derived(
    filtered
      ? { icon: 'activity' as const, title: 'No activity matches these filters' }
      : { icon: 'activity' as const, title: 'No activity yet' },
  );
</script>

{#snippet clearAction()}
  <Button onclick={clear}>Clear filters</Button>
{/snippet}

<PageHeader title="Activity" description="Everything brr did, newest first.">
  {#snippet actions()}
    <Button icon="download" loading={exporting} onclick={exportJson}>Export JSON</Button>
  {/snippet}
</PageHeader>

<div class="stack">
  <div class="filters">
    <div class="kind"><Select label="Type" bind:value={category} options={categoryOptions} /></div>
    <div class="subject"><Select label="Profile or browser" bind:value={focus} options={focusOptions} /></div>
    <div class="time"><Select label="Time" bind:value={time} options={timeOptions} /></div>
    <div class="clients"><Checkbox bind:checked={showClients} label="Show agent connections" /></div>
  </div>

  <Table
    caption="Activity"
    {columns}
    rows={list.items}
    key={(row: BrrEvent) => String(row.seq)}
    loading={list.loading}
    error={list.error}
    onretry={() => list.reload()}
    empty={{ ...empty, action: filtered ? clearAction : undefined }}
    footer={list.hasMore || list.capped ? more : undefined}
    expanded={(row: BrrEvent) => open.has(row.seq)}
    rowClass={(row: BrrEvent) => (fresh.has(row.seq) ? 'fresh' : undefined)}
  >
    {#snippet row(item: BrrEvent)}
      <td>
        <span class="time-cell">
          <Button
            variant="ghost"
            size="sm"
            iconOnly
            icon={open.has(item.seq) ? 'chevron-down' : 'chevron-right'}
            aria-label="Show event details"
            aria-expanded={open.has(item.seq)}
            onclick={() => toggle(item.seq)}
          />
          <time class="truncate" datetime={new Date(item.ts).toISOString()} title={absolute(item.ts)}>{relative(item.ts, clock.now)}</time>
        </span>
      </td>
      <td class="kind-cell truncate">{kindLabel(item.kind)}</td>
      <td class="truncate">
        <button type="button" class="subject-link" title="Show only activity about this" onclick={() => (focus = `${item.subject.type}:${item.subject.id}`)}>
          {subjectLabel(item.subject.type, item.subject.id)}
        </button>
      </td>
      <td class="truncate" title={summarize(item)}>{summarize(item)}</td>
    {/snippet}
    {#snippet detail(item: BrrEvent)}
      <CodeBlock code={JSON.stringify(item, null, 2)} label="Event data" />
    {/snippet}
  </Table>
</div>

{#snippet more()}
  <PagedFooter {list} />
{/snippet}

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .filters {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
  }

  .kind {
    width: 240px;
  }

  .subject {
    width: 280px;
  }

  .time {
    width: 180px;
  }

  .clients {
    display: flex;
    align-items: center;
    height: var(--control-h-md);
    white-space: nowrap;
  }

  .subject-link {
    max-width: 100%;
    padding: 0;
    overflow: hidden;
    background: none;
    border: 0;
    color: var(--text);
    font: inherit;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }

  .subject-link:hover {
    color: var(--accent);
    text-decoration: underline;
  }

  .time-cell {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-left: calc(var(--space-1) * -1);
    min-width: 0;
  }

  .kind-cell {
    color: var(--text-secondary);
  }

  :global(tr.fresh) {
    animation: fresh 1200ms var(--ease-standard); /* matches FADE_MS */
  }

  @keyframes fresh {
    from {
      background: var(--accent-subtle);
    }
    to {
      background: transparent;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    :global(tr.fresh) {
      animation: none;
    }
  }
</style>
