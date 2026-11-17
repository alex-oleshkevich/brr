<script lang="ts">
  import { bytes } from '$lib/format';
  import BarChart from '$lib/ui/BarChart.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { trafficByDay, type OverviewData } from './data.svelte';

  let { data }: { data: OverviewData } = $props();

  const SERIES = [{ key: 'bytes', label: 'Traffic' }];

  const days = $derived(trafficByDay(data.traffic.data));
  const points = $derived(
    days.map((d) => ({
      x: new Date(d.start).toLocaleDateString(undefined, { weekday: 'short' }),
      values: { bytes: d.bytes },
    })),
  );
  const total = $derived(days.reduce((sum, d) => sum + d.bytes, 0));
</script>

<Card title="Proxy traffic" description="Last 7 days">
  {#if data.traffic.error && !data.traffic.data}
    <EmptyState tone="danger" compact title="Couldn't load proxy traffic." description={data.traffic.error.message}>
      {#snippet action()}<Button size="sm" onclick={() => data.traffic.reload()}>Retry</Button>{/snippet}
    </EmptyState>
  {:else if data.traffic.loading}
    <Skeleton height="200px" radius="var(--radius-md)" />
  {:else if total === 0}
    <EmptyState compact title="No proxy traffic in the last 7 days." description="Only browsers that use a proxy or run in the sandbox are measured." />
  {:else}
    <BarChart label="Proxy traffic per day" series={SERIES} data={points} format={(n) => bytes(n)} />
    <p class="summary num">{bytes(total)} in total</p>
  {/if}
</Card>

<style>
  .summary {
    margin-top: var(--space-3);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }
</style>
