<script lang="ts">
  import { untrack } from 'svelte';
  import Select from '$lib/ui/Select.svelte';
  import Switch from '$lib/ui/Switch.svelte';
  import { isUnhealthy, proxyOptionLabel, unhealthyReason, type Proxy } from '../proxy';
  import type { SectionProps } from '../form';

  let { settings, readonly = false, proxies, pools }: SectionProps = $props();

  let proxy = $state(
    untrack(() => (settings.proxy === null ? '' : 'proxy_id' in settings.proxy ? `proxy:${settings.proxy.proxy_id}` : `pool:${settings.proxy.pool_id}`)),
  );
  let geoip = $state(untrack(() => (settings.geoip)));

  const proxyOptions = $derived([
    { value: '', label: 'None' },
    ...(proxies.length
      ? [{ group: 'Proxies', options: proxies.map((p) => ({ value: `proxy:${p.id}`, label: proxyOptionLabel(p) })) }]
      : []),
    ...(pools.length
      ? [{ group: 'Pools', options: pools.map((p) => ({ value: `pool:${p.id}`, label: `${p.name} · ${p.mode === 'sticky' ? 'Sticky' : 'Rotate'}` })) }]
      : []),
  ]);

  const chosen = $derived<Proxy | undefined>(proxy.startsWith('proxy:') ? proxies.find((p) => p.id === proxy.slice(6)) : undefined);

  $effect(() => {
    settings.proxy = proxy === '' ? null : proxy.startsWith('proxy:') ? { proxy_id: proxy.slice(6) } : { pool_id: proxy.slice(5) };
  });
  $effect(() => {
    settings.geoip = geoip;
  });
</script>

<div class="fields">
  <Select label="Proxy" bind:value={proxy} options={proxyOptions} disabled={readonly} />
  {#if chosen && isUnhealthy(chosen)}
    <p class="warning" role="status">This proxy is unhealthy: {unhealthyReason(chosen)}. Launches with it fail until it is fixed. <a href="/proxies">Open Proxies</a></p>
  {/if}
  <Switch label="Match location to proxy (GeoIP)" bind:checked={geoip} disabled={readonly} hint="Sets timezone, locale and geolocation from the proxy's exit IP." />
</div>

<style>
  .warning {
    margin: 0;
    color: var(--warning-fg);
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
</style>
