<script lang="ts">
  import { settingsHref } from '$lib/settings/sections';
  import { usd } from '$lib/format';
  import BarChart from '$lib/ui/BarChart.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import type { OverviewData } from './data.svelte';

  let { data }: { data: OverviewData } = $props();

  const SERIES = [{ key: 'spend', label: 'Spend' }];

  const days = $derived(data.stats.data ?? []);
  const points = $derived(
    days.map((d) => ({
      x: new Date(d.start_ms).toLocaleDateString(undefined, { weekday: 'short' }),
      values: { spend: d.spend_usd_micros / 1_000_000 },
    })),
  );
  const total = $derived(days.reduce((sum, d) => sum + d.spend_usd_micros, 0));
  const solves = $derived(days.reduce((sum, d) => sum + d.solves, 0));
  const failures = $derived(days.reduce((sum, d) => sum + d.failures, 0));
  const attempts = $derived(solves + failures);
</script>

<Card title="Captcha spend" description="Last 7 days">
  {#snippet actions()}<Button variant="ghost" size="sm" href={settingsHref('captcha')}>Captcha settings</Button>{/snippet}
  {#if data.stats.error && !data.stats.data}
    <EmptyState tone="danger" compact title="Couldn't load captcha spend." description={data.stats.error.message}>
      {#snippet action()}<Button size="sm" onclick={() => data.stats.reload()}>Retry</Button>{/snippet}
    </EmptyState>
  {:else if data.stats.loading}
    <Skeleton height="200px" radius="var(--radius-md)" />
  {:else if attempts === 0 && total === 0}
    <EmptyState compact title="No captchas were solved in the last 7 days." />
  {:else}
    <BarChart label="Captcha spend per day" series={SERIES} data={points} format={(n) => usd(n * 1_000_000)} />
    <p class="summary num">
      {usd(total)} spent · {solves} solved{attempts > 0 ? ` · ${Math.round((solves / attempts) * 100)}% success` : ''}
    </p>
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
