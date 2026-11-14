<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { SOURCE_NAMES, type BuildSource } from '$lib/builds/labels';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let { source, installedKeys, oninstall }: { source: BuildSource; installedKeys: Set<string>; oninstall: () => void } = $props();

  let pendingVersion = $state<string | null>(null);

  const available = liveQuery(
    async () => (await unwrap(api.GET('/v1/builds/available', { params: { query: { source } } }))).available,
    { kinds: [] },
  );

  const rows = $derived.by(() => {
    const byVersion = new Map<string, { version: string; channels: string[] }>();
    for (const item of available.data ?? []) {
      const row = byVersion.get(item.version) ?? { version: item.version, channels: [] };
      if (item.channel && !row.channels.includes(item.channel)) row.channels.push(item.channel);
      byVersion.set(item.version, row);
    }
    return [...byVersion.values()];
  });

  const buildMissing = $derived(available.error?.code === 'BuildMissing' ? available.error : null);

  async function install(version: string) {
    if (pendingVersion) return;
    pendingVersion = version;
    try {
      await unwrap(api.POST('/v1/builds', { body: { source, version } }));
      toast.success(`Installing ${SOURCE_NAMES[source]}`);
      oninstall();
    } catch (e) {
      toast.apiError(e, "Couldn't start the install");
    } finally {
      pendingVersion = null;
    }
  }
</script>

{#if buildMissing}
  <EmptyState tone="danger" title="Not available for this source" description={buildMissing.message}>
    {#snippet action()}<Button onclick={() => available.reload()}>Retry</Button>{/snippet}
  </EmptyState>
{:else}
  <Table
    caption="Available versions"
    columns={[
      { key: 'version', label: 'Version' },
      { key: 'channels', label: 'Channels', width: '30%' },
      { key: 'actions', label: 'Actions', width: '130px', srOnly: true, keepWidth: true },
    ]}
    {rows}
    key={(row) => row.version}
    loading={available.loading}
    error={available.error}
    onretry={() => available.reload()}
    empty={{ title: 'No versions available', description: 'This source offers nothing for this machine right now.' }}
  >
    {#snippet row(item)}
      <td class="num">{item.version}</td>
      <td>
        <span class="caps">
          {#each item.channels as channel (channel)}<Badge>{channel}</Badge>{/each}
        </span>
      </td>
      <td>
        {#if installedKeys.has(`${source}:${item.version}`)}
          <Badge tone="success">Installed</Badge>
        {:else}
          <Button variant="ghost" size="sm" aria-label="Install {item.version}" loading={pendingVersion === item.version} disabled={pendingVersion !== null} onclick={() => install(item.version)}>Install</Button>
        {/if}
      </td>
    {/snippet}
  </Table>
{/if}

<style>
  .caps {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    padding: var(--space-1) 0;
  }
</style>
