<script lang="ts">
  import { goto } from '$app/navigation';
  import { api, unwrap } from '$lib/api/client';
  import { session } from '$lib/api/session';
  import { liveQuery } from '$lib/api/live.svelte';
  import { pagedQuery } from '$lib/api/paged.svelte';
  import type { components } from '$lib/api/schema';
  import { buildLabel } from '$lib/builds/labels';
  import { clock } from '$lib/clock.svelte';
  import { bytes, duration, trafficTotal } from '$lib/format';
  import { canPromote, canStop, outcome, profileLabel } from '$lib/instances/actions';
  import LaunchDialog from '$lib/instances/LaunchDialog.svelte';
  import PromoteDialog from '$lib/instances/PromoteDialog.svelte';
  import StopDialog from '$lib/instances/StopDialog.svelte';
  import AttachDialog from '$lib/runs/AttachDialog.svelte';
  import { atLeastZero, templateLabel } from '$lib/runs/runs';
  import { ACTIVE_STATES } from '$lib/state';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import PagedFooter from '$lib/ui/PagedFooter.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Instance = components['schemas']['Instance'];

  const LIMIT = 100;
  const IDLE_STATES = ['running', 'idle', 'reaping'];

  let launching = $state(false);
  let attaching = $state(false);
  let promoteId = $state<string | null>(null);
  let promoting = $state(false);
  let stopTarget = $state<Instance | null>(null);
  let stopping = $state(false);

  const list = pagedQuery(
    async (cursor) => {
      const page = await unwrap(api.GET('/v1/instances', { params: { query: { state: ACTIVE_STATES.join(','), limit: LIMIT, cursor } } }));
      return { items: page.instances, next: page.next_cursor };
    },
    {
      kinds: ['instance.*', 'tab.opened', 'tab.closed', 'lease.*'],
      key: (row) => row.id,
      moreError: "Couldn't load more browsers",
      refreshMs: 15_000,
    },
  );

  const columns = [
    { key: 'profile', label: 'Profile', width: '15%' },
    { key: 'template', label: 'Template', width: '11%' },
    { key: 'build', label: 'Browser version', width: '15%' },
    { key: 'host', label: 'Host', width: '11%' },
    { key: 'display', label: 'Display', width: '84px' },
    { key: 'tabs', label: 'Tabs', align: 'right' as const, width: '64px' },
    { key: 'clients', label: 'Clients', width: '11%' },
    { key: 'idle', label: 'Idle for', width: '84px' },
    { key: 'memory', label: 'RAM', align: 'right' as const, width: '84px' },
    { key: 'traffic', label: 'Traffic', align: 'right' as const, width: '84px' },
    { key: 'state', label: 'State', width: '130px' },
    { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
  ];

  const hosts = liveQuery(async () => (await unwrap(api.GET('/v1/hosts'))).hosts, { kinds: ['instance.state_changed'] });
  const hostNames = $derived(new Map((hosts.data ?? []).map((h) => [h.id, h.name] as const)));
  const hostLabel = (item: Instance) => (item.host_id ? (hostNames.get(item.host_id) ?? 'Unknown host') : item.kind === 'live' ? '—' : 'This machine');

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
  const clients = (item: Instance) => [...(holders.get(item.id) ?? [])].join(', ') || '—';

  async function detach(item: Instance) {
    const ok = await confirm({
      title: 'Detach from this browser?',
      description: 'brr stops watching it. The browser keeps running.',
      confirmLabel: 'Detach',
    });
    if (!ok) return;
    try {
      await unwrap(api.POST('/v1/instances/{instance_id}/detach', { params: { path: { instance_id: item.id } }, body: {} }));
      toast.success('Detached');
      list.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't detach from the browser");
    }
  }
</script>

<PageHeader title="Browsers" description="Browsers brr is running or attached to. Click one to watch it or take control.">
  {#snippet actions()}
    <Button icon="plus" onclick={() => (attaching = true)}>Attach</Button>
    <Button variant="primary" icon="plus" onclick={() => (launching = true)}>Launch</Button>
  {/snippet}
</PageHeader>

<LaunchDialog bind:open={launching} />
<AttachDialog bind:open={attaching} />
{#if promoteId}<PromoteDialog bind:open={promoting} instanceId={promoteId} onpromoted={() => list.reload()} />{/if}
{#if stopTarget}<StopDialog bind:open={stopping} instance={stopTarget} onpromoted={() => list.reload()} />{/if}

{#snippet more()}
  <PagedFooter {list} />
{/snippet}

<Table
  caption="Browsers"
  {columns}
  rows={list.items}
  key={(row: Instance) => row.id}
  loading={list.loading}
  error={list.error}
  onretry={() => list.reload()}
  empty={{ icon: 'play', title: 'No running browsers', description: 'Use Launch to start one, or Attach to use a browser you already run.' }}
  rowClass={() => 'clickable'}
  footer={list.hasMore || list.capped ? more : undefined}
>
  {#snippet row(item: Instance)}
    {@const failed = outcome(item)}
    <td>
      <span class="profile">
        {#if item.kind === 'live'}
          <a href="/browsers/{item.id}"><Badge>Attached</Badge></a>
        {:else}
          <a class="truncate" href="/browsers/{item.id}" title={profileLabel(item)}>{profileLabel(item)}</a>
          {#if item.promote_to_name}<Badge tone="accent">Will be kept as {item.promote_to_name}</Badge>{/if}
        {/if}
      </span>
    </td>
    <td class="truncate" title={item.template_name ? templateLabel(item.template_name) : undefined}>{templateLabel(item.template_name)}</td>
    <td class="truncate" title={item.build ? buildLabel(item.build) : undefined}>{item.build ? buildLabel(item.build) : '—'}</td>
    <td class="truncate" title={hostLabel(item)}>{hostLabel(item)}</td>
    <td class="truncate">{item.display}</td>
    <td class="num right">{item.tab_count}</td>
    <td class="truncate" title={clients(item)}>{clients(item)}</td>
    <td class="num">{IDLE_STATES.includes(item.state) && item.last_activity_at ? duration(atLeastZero(clock.now - item.last_activity_at)) : '—'}</td>
    <td class="num right">{bytes(item.rss_bytes)}</td>
    <td class="num right" title={item.traffic ? undefined : 'Not measured'}>{item.traffic ? bytes(trafficTotal(item.traffic)) : '—'}</td>
    <td>{#if failed}<Badge dot tone={failed.tone}>{failed.label}</Badge>{:else}<StateBadge kind="instance" state={item.state} label={item.kind === 'live' && item.state === 'attached' ? 'Connected' : undefined} />{/if}</td>
    <td class="menu">
      <Menu
        label="Actions for {profileLabel(item)}"
        items={[
          { label: 'Open', onselect: () => goto('/browsers/' + item.id) },
          ...(canStop(item) ? [{ label: 'Stop…', onselect: () => ((stopTarget = item), (stopping = true)) }] : []),
          ...(canPromote(item) ? [{ label: "Keep this run's data…", onselect: () => ((promoteId = item.id), (promoting = true)) }] : []),
          ...(item.kind === 'live' ? [{ label: 'Detach…', onselect: () => detach(item) }] : []),
        ]}
      />
    </td>
  {/snippet}
</Table>

<style>
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

  .profile :global(.badge) {
    flex-shrink: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .profile a {
    flex-shrink: 0;
    max-width: 60%;
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
