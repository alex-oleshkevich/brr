<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { pagedQuery } from '$lib/api/paged.svelte';
  import type { components } from '$lib/api/schema';
  import { templateNameLabel } from '$lib/config/templates';
  import { endReason } from '$lib/events/summary';
  import ActivityPanel from '$lib/runs/ActivityPanel.svelte';
  import FilesPanel from '$lib/runs/FilesPanel.svelte';
  import RunTable from '$lib/runs/RunTable.svelte';
  import { FINAL_STATES } from '$lib/runs/runs';
  import { settingsHref } from '$lib/settings/sections';
  import Button from '$lib/ui/Button.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import PagedFooter from '$lib/ui/PagedFooter.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Tabs from '$lib/ui/Tabs.svelte';

  type Instance = components['schemas']['Instance'];
  type EndReason = components['schemas']['EndReason'];

  const LIMIT = 50;
  const DAY_MS = 86_400_000;
  const ALL = 'all';
  const TABS = [
    { id: 'runs', label: 'Runs' },
    { id: 'activity', label: 'Activity' },
    { id: 'files', label: 'Files' },
  ];
  const END_REASONS: EndReason[] = [
    'stop',
    'shutdown',
    'last_tab_closed',
    'cancel',
    'queue_timeout',
    'launch_error',
    'start_timeout',
    'crashed',
    'daemon_restart',
    'evicted',
    'memory_pressure',
    'reaped',
  ];

  const initial = page.url.searchParams;
  let tab = $state(TABS.find((t) => t.id === initial.get('tab'))?.id ?? 'runs');
  let profile = $state(initial.get('profile') ?? ALL);
  let template = $state(initial.get('template') ?? ALL);
  let reason = $state(initial.get('reason') ?? ALL);
  let from = $state(initial.get('from') ?? '');
  let until = $state(initial.get('until') ?? '');

  // The URL is the source of truth for the tab; picking one drops the other tab's filters from it.
  $effect(() => {
    const requested = page.url.searchParams.get('tab');
    tab = (TABS.find((t) => t.id === requested) ?? TABS[0]).id;
  });

  $effect(() => {
    if (tab !== (page.url.searchParams.get('tab') ?? TABS[0].id)) void goto(`?tab=${tab}`, { replaceState: true, noScroll: true, keepFocus: true });
  });

  $effect(() => {
    if (tab !== 'runs' || (page.url.searchParams.get('tab') ?? 'runs') !== 'runs') return;
    const url = new URL(page.url);
    for (const [key, value] of [['profile', profile === ALL ? '' : profile], ['template', template === ALL ? '' : template], ['reason', reason === ALL ? '' : reason], ['from', from], ['until', until]]) {
      if (value) url.searchParams.set(key, value);
      else url.searchParams.delete(key);
    }
    if (url.search !== page.url.search) void goto(url, { replaceState: true, noScroll: true, keepFocus: true });
  });

  const retention = liveQuery(() => unwrap(api.GET('/v1/retention')), { kinds: ['settings.changed'] });
  const profiles = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, { kinds: ['profile.*'] });
  const templates = liveQuery(async () => (await unwrap(api.GET('/v1/templates'))).templates, { kinds: ['template.*'] });

  const profileOptions = $derived([{ value: ALL, label: 'All profiles' }, ...(profiles.data ?? []).filter((p) => !p.builtin).map((p) => ({ value: p.id, label: p.name }))]);
  const templateOptions = $derived([{ value: ALL, label: 'All templates' }, ...(templates.data ?? []).map((t) => ({ value: t.id, label: templateNameLabel(t.name) }))]);
  const reasonOptions = [{ value: ALL, label: 'Any end reason' }, ...END_REASONS.map((r) => ({ value: r, label: endReason(r) }))];

  const startOfDay = (day: string) => new Date(`${day}T00:00:00`).getTime();

  const list = pagedQuery<Instance>(
    async (cursor) => {
      const out = await unwrap(
        api.GET('/v1/instances', {
          params: {
            query: {
              state: FINAL_STATES,
              limit: LIMIT,
              cursor,
              profile: profile === ALL ? undefined : profile,
              template: template === ALL ? undefined : template,
              end_reason: reason === ALL ? undefined : (reason as EndReason),
              since: from ? startOfDay(from) : undefined,
              until: until ? startOfDay(until) + DAY_MS - 1 : undefined,
            },
          },
        }),
      );
      return { items: out.instances, next: out.next_cursor };
    },
    { kinds: ['instance.state_changed'], key: (row) => row.id, moreError: "Couldn't load more runs" },
  );

  const inverted = $derived(from !== '' && until !== '' && from > until);
  const filtered = $derived(profile !== ALL || template !== ALL || reason !== ALL || from !== '' || until !== '');

  function clear() {
    profile = ALL;
    template = ALL;
    reason = ALL;
    from = '';
    until = '';
  }
</script>

<PageHeader title="History" description="Finished runs, everything brr did, and the files agents saved." />

{#if retention.data}
  <p class="notice">
    History and activity are kept for {retention.data.events_days} days.
    <a href={settingsHref('storage')}>Change</a>
  </p>
{/if}

<div class="stack">
  <Tabs items={TABS} bind:value={tab} label="History" idPrefix="history" />

  <div role="tabpanel" id="history-panel-{tab}" aria-labelledby="history-tab-{tab}">
    {#if tab === 'runs'}
      <div class="stack">
        <div class="filters">
          <Select label="Profile" bind:value={profile} options={profileOptions} />
          <Select label="Template" bind:value={template} options={templateOptions} />
          <Select label="End reason" bind:value={reason} options={reasonOptions} />
          <div class="field">
            <label class="field-label" for="history-from">From</label>
            <input class="control" id="history-from" type="date" bind:value={from} max={until || undefined} />
          </div>
          <div class="field">
            <label class="field-label" for="history-until">Until</label>
            <input class="control" id="history-until" type="date" bind:value={until} min={from || undefined} />
          </div>
        </div>
        {#if inverted}<p class="field-error" role="alert">"From" is after "Until", so no run can match.</p>{/if}
        <RunTable runs={inverted ? [] : list.items} loading={list.loading} error={list.error} onretry={() => list.reload()}>
          {#snippet empty()}
            {#if filtered}<Button onclick={clear}>Clear filters</Button>{/if}
          {/snippet}
        </RunTable>
        {#if list.hasMore || list.capped}
          <div class="more"><PagedFooter {list} /></div>
        {/if}
      </div>
    {:else if tab === 'activity'}
      <ActivityPanel />
    {:else}
      <FilesPanel />
    {/if}
  </div>
</div>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .notice {
    margin: calc(var(--space-2) * -1) 0 var(--space-4);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .filters {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-4);
  }

  .more {
    display: flex;
    justify-content: center;
  }

  .notice a {
    text-decoration: underline;
  }
</style>
