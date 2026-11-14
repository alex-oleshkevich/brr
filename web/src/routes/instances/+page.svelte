<script lang="ts">
  import PagedFooter from '$lib/ui/PagedFooter.svelte';
  import { api, unwrap } from '$lib/api/client';
  import { session } from '$lib/api/session';
  import { liveQuery } from '$lib/api/live.svelte';
  import { pagedQuery } from '$lib/api/paged.svelte';
  import type { components } from '$lib/api/schema';
  import { clock } from '$lib/clock.svelte';
  import { ACTIVE_STATES, ENDED_STATES } from '$lib/state';
  import { bytes, duration } from '$lib/format';
  import { goto } from '$app/navigation';
  import { buildLabel } from '$lib/builds/labels';
  import { canPromote, canRelaunch, canStop, outcome, profileLabel, whyEnded } from '$lib/instances/actions';
  import StopDialog from '$lib/instances/StopDialog.svelte';
  import { absolute } from '$lib/format';
  import LaunchDialog from '$lib/instances/LaunchDialog.svelte';
  import PromoteDialog from '$lib/instances/PromoteDialog.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';
  import Table from '$lib/ui/Table.svelte';
  import Tabs from '$lib/ui/Tabs.svelte';

  type Instance = components['schemas']['Instance'];

  const STATES: Record<string, string> = { active: ACTIVE_STATES.join(','), ended: ENDED_STATES.join(',') };
  const LIMIT = 100;
  const IDLE_STATES = ['running', 'idle', 'reaping'];

  let tab = $state('active');
  let launching = $state(false);
  let promoteId = $state<string | null>(null);
  let promoting = $state(false);
  let stopTarget = $state<Instance | null>(null);
  let stopping = $state(false);
  let relaunchId = $state<string | null>(null);
  let relaunching = $state(false);

  const list = pagedQuery(
    async (cursor) => {
      const page = await unwrap(api.GET('/v1/instances', { params: { query: { state: STATES[tab], limit: LIMIT, cursor } } }));
      return { items: page.instances, next: page.next_cursor };
    },
    {
      kinds: ['instance.*', 'tab.opened', 'tab.closed', 'lease.*'],
      key: (row) => row.id,
      moreError: "Couldn't load more browsers",
      refreshMs: 15_000,
    },
  );

  const counts = liveQuery(
    async () => {
      const [active, ended] = await Promise.all(
        ['active', 'ended'].map((key) => unwrap(api.GET('/v1/instances', { params: { query: { state: STATES[key], limit: LIMIT } } }))),
      );
      const label = (page: typeof active) => (page.next_cursor ? `${LIMIT}+` : String(page.instances.length));
      return { active: label(active), ended: label(ended) };
    },
    { kinds: ['instance.state_changed'] },
  );

  const NAME_COLUMN = { key: 'profile', label: 'Browser', width: '22%' };
  const ACTIONS_COLUMN = { key: 'actions', label: 'Actions', width: '48px', srOnly: true };
  const activeColumns = [
    NAME_COLUMN,
    { key: 'build', label: 'Browser version', width: '24%' },
    { key: 'state', label: 'State', width: '140px' },
    { key: 'display', label: 'Display', width: '88px' },
    { key: 'tabs', label: 'Tabs', align: 'right' as const, width: '72px' },
    { key: 'use', label: 'In use by', width: '16%' },
    { key: 'idle', label: 'Idle for', width: '96px' },
    { key: 'memory', label: 'Memory', align: 'right' as const, width: '88px' },
    ACTIONS_COLUMN,
  ];
  const endedColumns = [
    NAME_COLUMN,
    { key: 'build', label: 'Browser version', width: '24%' },
    { key: 'state', label: 'State', width: '140px' },
    { key: 'endedAt', label: 'Ended at', width: '150px' },
    { key: 'reason', label: 'Reason' },
    { key: 'duration', label: 'Duration', width: '90px' },
    ACTIONS_COLUMN,
  ];
  const columns = $derived(tab === 'ended' ? endedColumns : activeColumns);

  const ranFor = (item: Instance) => (item.started_at && item.ended_at ? duration(item.ended_at - item.started_at) : '—');
  // Who holds each browser, from its active leases.
  const leases = liveQuery(async () => (await unwrap(api.GET('/v1/leases', {}))).leases, { kinds: ['lease.*'] });
  const holders = $derived.by(() => {
    const byInstance = new Map<string, Set<string>>();
    for (const lease of leases.data ?? []) {
      if (lease.released_at != null) continue;
      const label = lease.client_id === session.clientId ? 'You' : lease.client_label;
      byInstance.set(lease.instance_id, (byInstance.get(lease.instance_id) ?? new Set()).add(label));
    }

    return byInstance;
  });
  const inUse = (item: Instance) => [...(holders.get(item.id) ?? [])].join(', ') || '—';

  const tabItems = $derived([
    { id: 'active', label: 'Active', count: counts.data?.active },
    { id: 'ended', label: 'Ended', count: counts.data?.ended },
  ]);

  const empty = $derived(
    tab === 'active'
      ? { icon: 'play' as const, title: 'No running browsers', description: 'Use Launch above to start one.' }
      : { icon: 'play' as const, title: 'No ended browsers yet' },
  );
</script>

<PageHeader title="Browsers" description="Browsers brr is running or attached to. Click one to watch it or take control.">
  {#snippet actions()}
    <Button variant="primary" icon="plus" onclick={() => (launching = true)}>Launch</Button>
  {/snippet}
</PageHeader>

<LaunchDialog bind:open={launching} />
{#if promoteId}<PromoteDialog bind:open={promoting} instanceId={promoteId} />{/if}
{#if relaunchId}<LaunchDialog bind:open={relaunching} profileId={relaunchId} />{/if}
{#if stopTarget}<StopDialog bind:open={stopping} instance={stopTarget} />{/if}

{#snippet more()}
  <PagedFooter {list} />
{/snippet}

<div class="stack">
  <Tabs items={tabItems} bind:value={tab} label="Browser state" idPrefix="instances" />

  <div role="tabpanel" id="instances-panel-{tab}" aria-labelledby="instances-tab-{tab}">
    <Table
      caption={tab === 'ended' ? 'Ended browsers' : 'Running browsers'}
      {columns}
      rows={list.items}
      key={(row: Instance) => row.id}
      loading={list.loading}
      error={list.error}
      onretry={() => list.reload()}
      {empty}
      rowClass={() => 'clickable'}
      footer={list.hasMore || list.capped ? more : undefined}
    >
      {#snippet row(item: Instance)}
        <td>
          <span class="profile">
            <a class="truncate" href="/instances/{item.id}" title={profileLabel(item)}>{profileLabel(item)}</a>
            {#if item.kind === 'live'}<Badge>live</Badge>{/if}
          </span>
        </td>
        <td class="truncate" title={item.build ? buildLabel(item.build) : undefined}>{item.build ? buildLabel(item.build) : '—'}</td>
        {@const failed = outcome(item)}
        <td>{#if failed}<Badge dot tone={failed.tone}>{failed.label}</Badge>{:else}<StateBadge kind="instance" state={item.state} />{/if}</td>
        {#if tab === 'ended'}
          <td class="num">{item.ended_at ? absolute(item.ended_at) : '—'}</td>
          <td class="truncate" title={whyEnded(item)}>{whyEnded(item)}</td>
          <td class="num">{ranFor(item)}</td>
        {:else}
        <td class="truncate">{item.display}</td>
        <td class="num right">{item.tab_count}</td>
        <td class="truncate" title={inUse(item)}>{inUse(item)}</td>
        <td class="num">{IDLE_STATES.includes(item.state) && item.last_activity_at ? duration(clock.now - item.last_activity_at) : '—'}</td>
        <td class="num right">{bytes(item.rss_bytes)}</td>
        {/if}
        <td class="menu">
          <Menu
            label="Actions for {profileLabel(item)}"
            items={[
              { label: 'Open', onselect: () => goto('/instances/' + item.id) },
              ...(canRelaunch(item) ? [{ label: 'Launch again…', onselect: () => ((relaunchId = item.profile_id!), (relaunching = true)) }] : []),
              ...(canStop(item) ? [{ label: 'Stop…', onselect: () => ((stopTarget = item), (stopping = true)) }] : []),
              ...(canPromote(item) ? [{ label: 'Promote…', onselect: () => ((promoteId = item.id), (promoting = true)) }] : []),
            ]}
          />
        </td>
      {/snippet}
    </Table>
  </div>
</div>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .profile {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  /* The whole row opens the browser: the name link stretches over it, and the menu stays above. */
  :global(tr.clickable) {
    position: relative;
    cursor: pointer;
  }

  .profile a::after {
    content: '';
    position: absolute;
    inset: 0;
  }

  .menu {
    position: relative;
    z-index: 1;
  }
</style>
