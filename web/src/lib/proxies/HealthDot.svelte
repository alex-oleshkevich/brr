<script lang="ts">
  import type { Proxy } from './format';
  import StatusDot from '$lib/ui/StatusDot.svelte';

  let { proxy, checking = false }: { proxy: Pick<Proxy, 'healthy'>; checking?: boolean } = $props();
</script>

{#if checking}
  <StatusDot tone="neutral" label="Checking…" pulse />
{:else if proxy.healthy === true}
  <StatusDot tone="success" label="Healthy" />
{:else if proxy.healthy === false}
  <StatusDot tone="danger" label="Unhealthy" />
{:else}
  <StatusDot tone="neutral" label="Not checked" />
{/if}
