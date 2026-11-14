<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { events } from '$lib/api/events.svelte';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components } from '$lib/api/schema';
  import { clock } from '$lib/clock.svelte';
  import { relative } from '$lib/format';
  import InUseDialog from '$lib/proxies/InUseDialog.svelte';
  import HealthDot from '$lib/proxies/HealthDot.svelte';
  import ImportDialog from '$lib/proxies/ImportDialog.svelte';
  import PoolDialog from '$lib/proxies/PoolDialog.svelte';
  import ProxyDialog from '$lib/proxies/ProxyDialog.svelte';
  import { proxyLabel, proxyUrl, type Proxy } from '$lib/proxies/format';
  import Button from '$lib/ui/Button.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import Table from '$lib/ui/Table.svelte';
  import Tabs from '$lib/ui/Tabs.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Pool = components['schemas']['Pool'];
  type Profile = components['schemas']['Profile'];

  // Health arrives as proxy.health events that patch rows; everything else reloads after its mutation.
  const PAGE_LIMIT = 1000;

  async function allProxies() {
    const all: Proxy[] = [];
    let cursor: string | undefined;
    do {
      const page = await unwrap(api.GET('/v1/proxies', { params: { query: { limit: PAGE_LIMIT, cursor } } }));
      all.push(...page.proxies);
      cursor = page.next_cursor ?? undefined;
    } while (cursor);
    return all;
  }

  async function allPools() {
    const all: Pool[] = [];
    let cursor: string | undefined;
    do {
      const page = await unwrap(api.GET('/v1/proxy-pools', { params: { query: { limit: PAGE_LIMIT, cursor } } }));
      all.push(...page.pools);
      cursor = page.next_cursor ?? undefined;
    } while (cursor);
    return all;
  }

  const proxyList = liveQuery(allProxies, { kinds: [] });
  const poolList = liveQuery(allPools, { kinds: [] });

  let proxies = $state<Proxy[]>([]);
  // proxy id -> when its check started; a row stops showing "Checking…" once a newer checked_at is loaded.
  let checking = $state(new Map<string, number>());
  let tab = $state('proxies');

  const mark = (id: string, on: boolean) => {
    const next = new Map(checking);
    if (on) next.set(id, Date.now());
    else next.delete(id);
    checking = next;
  };

  const isChecking = (p: Proxy) => {
    const started = checking.get(p.id);
    return started !== undefined && !(p.checked_at && p.checked_at >= started - 1000);
  };

  $effect(() => {
    proxies = proxyList.data ?? [];
  });

  $effect(() =>
    events.on(['proxy.health'], (e) => {
      const data = e.data as { proxy_id: string; healthy: boolean; exit_ip?: string; country?: string; latency_ms?: number };
      // A new proxy's first check can finish before the list that contains it has loaded.
      if (!proxies.some((p) => p.id === data.proxy_id)) proxyList.reload();
      // The event carries no failure cause; a reload brings last_error with it.
      if (!data.healthy) proxyList.reload();
      proxies = proxies.map((p) =>
        p.id === data.proxy_id
          ? { ...p, healthy: data.healthy, exit_ip: data.exit_ip ?? null, country: data.country ?? null, latency_ms: data.latency_ms ?? null, checked_at: e.ts }
          : p,
      );
      mark(data.proxy_id, false);
    }),
  );


  let proxyDialog = $state(false);
  let editing = $state<Proxy | undefined>();
  let importing = $state(false);
  let poolDialog = $state(false);
  let editingPool = $state<Pool | undefined>();
  let inUse = $state<{ proxy: Proxy; users: Profile[] } | null>(null);
  let inUseOpen = $state(false);

  // "· connection refused · checked 2m ago": why a proxy is unhealthy, and how fresh that verdict is.
  const healthDetail = (proxy: Proxy) =>
    [proxy.healthy === false ? proxy.last_error : null, proxy.checked_at ? `checked ${relative(proxy.checked_at, clock.now)}` : null]
      .filter(Boolean)
      .map((part) => ` · ${part}`)
      .join('');

  const poolsOf = (proxy: Proxy) => (poolList.data ?? []).filter((p) => p.proxy_ids.includes(proxy.id));

  async function check(proxy: Proxy) {
    mark(proxy.id, true);
    try {
      const updated = await unwrap(api.POST('/v1/proxies/{proxy_id}/check', { params: { path: { proxy_id: proxy.id } }, body: {} }));
      proxies = proxies.map((p) => (p.id === updated.id ? updated : p));
    } catch (e) {
      toast.apiError(e, "Couldn't check the proxy");
    } finally {
      mark(proxy.id, false);
    }
  }

  // Profiles that point straight at this proxy; the daemon refuses to delete it while any exist.
  async function profilesUsing(proxy: Proxy) {
    const { profiles } = await unwrap(api.GET('/v1/profiles'));
    return profiles.filter((p) => {
      const choice = p.settings.proxy;
      return choice !== null && 'proxy_id' in choice && choice.proxy_id === proxy.id;
    });
  }

  async function remove(proxy: Proxy) {
    try {
      const users = await profilesUsing(proxy);
      if (users.length) {
        inUse = { proxy, users };
        inUseOpen = true;
        return;
      }
    } catch (e) {
      toast.apiError(e, "Couldn't check where the proxy is used");
      return;
    }
    const pools = poolsOf(proxy);
    const ok = await confirm({
      title: `Delete ${proxyLabel(proxy)}?`,
      description: pools.length ? `It is removed from ${pools.length === 1 ? 'pool' : 'pools'} ${pools.map((p) => p.name).join(', ')}.` : "This can't be undone.",
      confirmLabel: 'Delete proxy',
      tone: 'danger',
    });
    if (!ok) return;

    try {
      await unwrap(api.DELETE('/v1/proxies/{proxy_id}', { params: { path: { proxy_id: proxy.id } } }));
      toast.success('Proxy deleted');
      proxyList.reload();
      poolList.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't delete the proxy");
    }
  }

  async function removePool(pool: Pool) {
    const ok = await confirm({
      title: `Delete pool ${pool.name}?`,
      description: "The proxies stay. This can't be undone.",
      confirmLabel: 'Delete pool',
      tone: 'danger',
    });
    if (!ok) return;

    try {
      await unwrap(api.DELETE('/v1/proxy-pools/{pool}', { params: { path: { pool: pool.id } } }));
      toast.success('Pool deleted');
      poolList.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't delete the pool");
    }
  }

  // Create and edit both start a check; the row shows it until proxy.health arrives or a checked_at is loaded.
  function saved(proxy: Proxy) {
    mark(proxy.id, true);
    proxyList.reload();
  }

  const members = (pool: Pool) => {
    const healthy = pool.proxy_ids.filter((id) => proxies.find((p) => p.id === id)?.healthy === true).length;
    return `${healthy} of ${pool.proxy_ids.length} healthy`;
  };

  const proxyColumns = [
    { key: 'proxy', label: 'Proxy', width: '24%' },
    { key: 'login', label: 'Login', width: '12%' },
    { key: 'health', label: 'Health' },
    { key: 'exit', label: 'Exit IP', width: '120px' },
    { key: 'country', label: 'Country', width: '76px' },
    { key: 'latency', label: 'Latency', width: '84px', align: 'right' as const },
    { key: 'actions', label: 'Actions', width: '176px', srOnly: true, keepWidth: true },
  ];
  const poolColumns = [
    { key: 'name', label: 'Name', width: '30%' },
    { key: 'mode', label: 'Mode', width: '30%' },
    { key: 'members', label: 'Members' },
    { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
  ];

  const tabItems = $derived([
    { id: 'proxies', label: 'Proxies', count: proxyList.data?.length },
    { id: 'pools', label: 'Pools', count: poolList.data?.length },
  ]);
</script>

<PageHeader title="Proxies" description="Proxies and pools that profiles can send their traffic through.">
  {#snippet actions()}
    <Button onclick={() => (importing = true)}>Bulk import</Button>
    <Button
      variant="primary"
      icon="plus"
      onclick={() => ((editing = undefined), (proxyDialog = true))}>Add proxy</Button>
  {/snippet}
</PageHeader>

<div class="stack">
  <Tabs items={tabItems} bind:value={tab} label="Proxies and pools" idPrefix="proxies" />

  <div role="tabpanel" id="proxies-panel-{tab}" aria-labelledby="proxies-tab-{tab}">
    {#if tab === 'proxies'}
      <Table
        caption="Proxies"
        columns={proxyColumns}
        rows={proxyList.data && proxies}
        key={(row: Proxy) => row.id}
        loading={proxyList.loading}
        error={proxyList.error}
        onretry={() => proxyList.reload()}
        empty={{ icon: 'globe', title: 'No proxies yet', description: 'Add one, or paste a list with Bulk import.' }}
      >
        {#snippet row(proxy: Proxy)}
          <td class="truncate" title={proxyUrl(proxy)}>{proxyUrl(proxy)}</td>
          <td class="truncate" title={proxy.username ?? undefined}>{proxy.username ?? '—'}</td>
          <td class="health" title={proxy.last_error ?? undefined}>
            <HealthDot {proxy} checking={isChecking(proxy)} />
            {#if !isChecking(proxy)}
              <span class="cause">{healthDetail(proxy)}</span>
            {/if}
          </td>
          <td class="num truncate">{proxy.exit_ip ?? '—'}</td>
          <td>{proxy.country ?? '—'}</td>
          <td class="num right">{proxy.latency_ms === null ? '—' : `${proxy.latency_ms} ms`}</td>
          <td>
            <div class="actions">
              <Button size="sm" loading={isChecking(proxy)} onclick={() => check(proxy)}>Check now</Button>
              <Menu
                label="Actions for {proxyLabel(proxy)}"
                items={[
                  { label: 'Edit…', onselect: () => ((editing = proxy), (proxyDialog = true)) },
                  { label: 'Delete…', tone: 'danger', onselect: () => remove(proxy) },
                ]}
              />
            </div>
          </td>
        {/snippet}
      </Table>
    {:else}
      <div class="pool-head">
        <Button icon="plus" onclick={() => ((editingPool = undefined), (poolDialog = true))}>New pool</Button>
      </div>
      <Table
        caption="Proxy pools"
        columns={poolColumns}
        rows={poolList.data}
        key={(row: Pool) => row.id}
        loading={poolList.loading}
        error={poolList.error}
        onretry={() => poolList.reload()}
        empty={{ icon: 'globe', title: 'No pools yet', description: 'Group proxies to hand them out per profile or per launch.' }}
      >
        {#snippet row(pool: Pool)}
          <td class="truncate" title={pool.name}>{pool.name}</td>
          <td>{pool.mode === 'sticky' ? 'Sticky per profile' : 'Rotate per launch'}</td>
          <td>{members(pool)}</td>
          <td>
            <Menu
              label="Actions for {pool.name}"
              items={[
                { label: 'Edit…', onselect: () => ((editingPool = pool), (poolDialog = true)) },
                { label: 'Delete…', tone: 'danger', onselect: () => removePool(pool) },
              ]}
            />
          </td>
        {/snippet}
      </Table>
    {/if}
  </div>
</div>

<ProxyDialog bind:open={proxyDialog} proxy={editing} onsaved={saved} />
<ImportDialog bind:open={importing} onimported={() => proxyList.reload()} />
<InUseDialog
  bind:open={inUseOpen}
  proxy={inUse?.proxy ?? null}
  users={inUse?.users ?? []}
  ondeleted={() => (proxyList.reload(), poolList.reload())}
/>
<PoolDialog bind:open={poolDialog} pool={editingPool} {proxies} onsaved={() => poolList.reload()} />

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .cause {
    color: var(--text-secondary);
  }

  :global(tbody tr td.health) {
    height: auto;
    min-height: var(--row-h);
    padding-block: var(--space-2);
    white-space: normal;
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .pool-head {
    display: flex;
    justify-content: flex-end;
    margin-bottom: var(--space-3);
  }
</style>
