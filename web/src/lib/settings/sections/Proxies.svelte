<script lang="ts">
  import { untrack } from 'svelte';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { events } from '$lib/api/events.svelte';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components } from '$lib/api/schema';
  import { clock } from '$lib/clock.svelte';
  import { bytes, relative } from '$lib/format';
  import { canonical } from '$lib/config/form';
  import InUseDialog from '$lib/proxies/InUseDialog.svelte';
  import HealthDot from '$lib/proxies/HealthDot.svelte';
  import ImportDialog from '$lib/proxies/ImportDialog.svelte';
  import PoolDialog from '$lib/proxies/PoolDialog.svelte';
  import ProxyDialog from '$lib/proxies/ProxyDialog.svelte';
  import { proxyLabel, proxyUrl, type Proxy } from '$lib/proxies/format';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import Table from '$lib/ui/Table.svelte';
  import Tabs from '$lib/ui/Tabs.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Pool = components['schemas']['Pool'];
  type Profile = components['schemas']['Profile'];
  type Template = components['schemas']['Template'];

  const PAGE_LIMIT = 1000;
  const TRAFFIC_DAYS = 30;

  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();

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
  const traffic = liveQuery(
    async () => {
      const { items } = await unwrap(api.GET('/v1/usage/traffic', { params: { query: { group: 'proxy', days: TRAFFIC_DAYS } } }));
      return new Map(items.map((item) => [item.id, item.sent_bytes + item.received_bytes]));
    },
    { kinds: ['instance.state_changed'] },
  );
  const poolList = liveQuery(allPools, { kinds: [] });
  const settings = liveQuery(() => unwrap(api.GET('/v1/settings')), { kinds: ['settings.changed'] });

  let proxies = $state<Proxy[]>([]);
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
      if (!proxies.some((p) => p.id === data.proxy_id)) proxyList.reload();
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
  let poolInUse = $state<{ pool: Pool; profiles: Profile[]; templates: Template[] } | null>(null);
  let poolInUseOpen = $state(false);

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
    let profiles: Profile[];
    let templates: Template[];
    try {
      const [profilePage, templatePage] = await Promise.all([unwrap(api.GET('/v1/profiles')), unwrap(api.GET('/v1/templates'))]);
      const usesPool = (choice: Profile['settings']['proxy']) => choice !== null && 'pool_id' in choice && choice.pool_id === pool.id;
      profiles = profilePage.profiles.filter((p) => usesPool(p.settings.proxy));
      templates = templatePage.templates.filter((t) => usesPool(t.settings.proxy));
    } catch (e) {
      toast.apiError(e, "Couldn't check where the pool is used");
      return;
    }
    if (profiles.length || templates.length) {
      poolInUse = { pool, profiles, templates };
      poolInUseOpen = true;
      return;
    }
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

  function saved(proxy: Proxy) {
    mark(proxy.id, true);
    proxyList.reload();
  }

  const members = (pool: Pool) => {
    const healthy = pool.proxy_ids.filter((id) => proxies.find((p) => p.id === id)?.healthy === true).length;
    return `${healthy} of ${pool.proxy_ids.length} healthy`;
  };

  const proxyColumns = [
    { key: 'proxy', label: 'Proxy', width: '22%' },
    { key: 'login', label: 'Login', width: '11%' },
    { key: 'health', label: 'Health' },
    { key: 'exit', label: 'Exit IP', width: '13%' },
    { key: 'country', label: 'Country', width: '9%' },
    { key: 'latency', label: 'Latency', width: '10%', align: 'right' as const },
    { key: 'traffic', label: 'Traffic (30 days)', width: '12%', align: 'right' as const },
    { key: 'actions', label: 'Actions', width: '150px', srOnly: true, keepWidth: true },
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

  let healthInterval = $state<number | null>(null);
  let checkUrl = $state('');
  let snapshot = $state('');
  let saving = $state(false);
  let serverError = $state<string | null>(null);

  const form = $derived(canonical({ healthInterval, checkUrl }));
  const advancedDirty = $derived(snapshot !== '' && form !== snapshot);
  $effect(() => {
    dirty = advancedDirty;
  });

  const sources = $derived(settings.data?.sources ?? {});
  const isEnv = (key: string) => sources[key] === 'env';
  const envName = (key: string) => 'BRR_' + key.replace('.', '__').toUpperCase();
  const badge = (key: string) => (isEnv(key) ? `Set by ${envName(key)}` : undefined);
  const envHint = (key: string, hint?: string) => (isEnv(key) ? 'Remove the environment variable to edit it here.' : hint);
  const checkUrlError = $derived.by(() => {
    const value = checkUrl.trim();
    if (!value) return 'Required';
    try {
      const url = new URL(value);
      return url.protocol === 'http:' || url.protocol === 'https:' ? null : 'Enter an http(s) URL.';
    } catch {
      return 'Enter an http(s) URL.';
    }
  });
  const invalid = $derived(healthInterval === null || !Number.isInteger(healthInterval) || healthInterval < 0 || checkUrlError !== null);

  function applySettings() {
    const s = settings.data?.settings;
    if (!s) return;
    healthInterval = s.proxy.health_interval_min;
    checkUrl = s.proxy.check_url;
    snapshot = canonical({ healthInterval, checkUrl });
    serverError = null;
  }

  $effect(() => {
    const next = settings.data;
    if (!next) return;
    if (untrack(() => advancedDirty)) return;
    untrack(applySettings);
  });

  async function saveAdvanced() {
    if (saving || !advancedDirty || invalid) return;
    saving = true;
    serverError = null;
    try {
      const before = JSON.parse(snapshot) as { healthInterval: number; checkUrl: string };
      const patch: Record<string, unknown> = {};
      if (!isEnv('proxy.health_interval_min') && healthInterval !== before.healthInterval) patch['proxy.health_interval_min'] = healthInterval;
      if (!isEnv('proxy.check_url') && checkUrl !== before.checkUrl) patch['proxy.check_url'] = checkUrl;
      if (Object.keys(patch).length > 0) await unwrap(api.PATCH('/v1/settings', { body: patch }));
      snapshot = form;
      settings.reload();
      toast.success('Settings saved');
    } catch (e) {
      if (e instanceof ApiError) serverError = e.message;
      else toast.apiError(e, "Couldn't save the settings");
    } finally {
      saving = false;
    }
  }
</script>

<div class="stack">
  <div class="head">
    <Button onclick={() => (importing = true)}>Bulk import</Button>
    <Button variant="primary" icon="plus" onclick={() => ((editing = undefined), (proxyDialog = true))}>Add proxy</Button>
  </div>

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
          <td class="truncate">{proxy.country ?? '—'}</td>
          <td class="num right">{proxy.latency_ms === null ? '—' : `${proxy.latency_ms} ms`}</td>
          <td class="num right">{bytes(traffic.data?.get(proxy.id) ?? 0)}</td>
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

  <Card title="Health checks">
    {#if settings.loading && snapshot === ''}
      <div aria-busy="true"><Skeleton height="64px" radius="var(--radius-md)" /></div>
    {:else if settings.error && snapshot === ''}
      <EmptyState tone="danger" title="Couldn't load proxy settings" description={settings.error.message}>
        {#snippet action()}<Button onclick={() => settings.reload()}>Retry</Button>{/snippet}
      </EmptyState>
    {:else}
      <div class="pair">
        <Input name="proxy.health_interval_min" label="Check proxies every (min)" type="number" min={0} bind:value={healthInterval} readonly={isEnv('proxy.health_interval_min')} badge={badge('proxy.health_interval_min')} hint={envHint('proxy.health_interval_min', 'How often each proxy is checked. 0 turns checks off.')} error={healthInterval === null || !Number.isInteger(healthInterval) || healthInterval < 0 ? 'Enter a whole number, 0 or more.' : serverError} />
        <Input name="proxy.check_url" label="Check URL" type="url" bind:value={checkUrl} readonly={isEnv('proxy.check_url')} badge={badge('proxy.check_url')} hint={envHint('proxy.check_url', 'Fetched through each proxy to test it and find its exit address.')} error={checkUrlError} />
      </div>
    {/if}
  </Card>
</div>
<SaveBar dirty={advancedDirty} {saving} disabled={invalid} onsave={saveAdvanced} ondiscard={applySettings} />

<ProxyDialog bind:open={proxyDialog} proxy={editing} onsaved={saved} />
<ImportDialog bind:open={importing} onimported={() => proxyList.reload()} />
<InUseDialog
  bind:open={inUseOpen}
  proxy={inUse?.proxy ?? null}
  users={inUse?.users ?? []}
  ondeleted={() => (proxyList.reload(), poolList.reload())}
/>
<PoolDialog bind:open={poolDialog} pool={editingPool} {proxies} onsaved={() => poolList.reload()} />
<Dialog
  bind:open={poolInUseOpen}
  size="sm"
  title="Pool {poolInUse?.pool.name ?? ''} is in use"
  description="This pool can't be deleted while a profile or template sends traffic through it. Change those to another proxy or pool first."
>
  {#if poolInUse}
    <div class="in-use">
      {#if poolInUse.profiles.length}
        <p class="in-use-label">Profiles</p>
        <ul class="users">
          {#each poolInUse.profiles as profile (profile.id)}
            <li><a href="/profiles/{profile.id}">{profile.name}</a></li>
          {/each}
        </ul>
      {/if}
      {#if poolInUse.templates.length}
        <p class="in-use-label">Templates</p>
        <ul class="users">
          {#each poolInUse.templates as template (template.id)}
            <li><a href="/templates/{template.id}">{template.name}</a></li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
  {#snippet footer()}
    <Button onclick={() => (poolInUseOpen = false)}>Close</Button>
  {/snippet}
</Dialog>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .head,
  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
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

  .pool-head {
    display: flex;
    justify-content: flex-end;
    margin-bottom: var(--space-3);
  }

  .pair {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-3);
  }

  @media (max-width: 1023px) {
    .pair {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .in-use {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .in-use-label {
    margin: 0;
    font-weight: 500;
  }

  .users {
    margin: 0;
    padding-left: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
</style>
