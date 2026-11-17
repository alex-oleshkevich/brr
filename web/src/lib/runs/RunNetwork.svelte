<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { fetchAll } from '$lib/api/paged.svelte';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components } from '$lib/api/schema';
  import { absolute } from '$lib/format';
  import { proxyLabel } from '$lib/proxies/format';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import KeyValue from '$lib/ui/KeyValue.svelte';
  import PanelState from './PanelState.svelte';
  import { NOT_RECORDED } from './runs';

  type Instance = components['schemas']['Instance'];

  let { run }: { run: Instance } = $props();

  const proxies = liveQuery(
    async () => {
      if (!run.proxy_id) return [];
      return fetchAll(async (cursor) => {
        const out = await unwrap(api.GET('/v1/proxies', { params: { query: { cursor } } }));
        return { items: out.proxies, next: out.next_cursor };
      });
    },
    { kinds: ['proxy.*'] },
  );

  const proxy = $derived((proxies.data ?? []).find((p) => p.id === run.proxy_id));
</script>

{#if run.kind === 'live'}
  <EmptyState icon="file" title="None" description="brr didn't start this browser, so it doesn't know what network it uses." compact />
{:else if !run.proxy_id && !run.settings}
  <EmptyState icon="file" title={NOT_RECORDED} compact />
{:else if !run.proxy_id}
  <EmptyState icon="file" title="No proxy" description="This run connected directly." compact />
{:else}
  <PanelState loading={proxies.loading} error={proxies.error} onretry={() => proxies.reload()} title="the proxy">
    {#if proxy}
      <KeyValue
        items={[
          { label: 'Proxy', value: proxyLabel(proxy), mono: true },
          { label: 'Exit IP', value: proxy.exit_ip ?? null, mono: true },
          { label: 'Country', value: proxy.country ?? null },
          { label: 'Last checked', value: proxy.checked_at ? absolute(proxy.checked_at) : null },
        ]}
      />
    {:else}
      <EmptyState icon="file" title="This proxy has since been deleted" compact />
    {/if}
  </PanelState>
{/if}
