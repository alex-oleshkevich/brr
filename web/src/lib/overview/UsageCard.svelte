<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import AreaChart from '$lib/ui/AreaChart.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';

  type Window = '1d' | '7d';

  const SERIES = [
    { key: 'chromium', label: 'Chromium' },
    { key: 'camoufox', label: 'Camoufox' },
    { key: 'attached', label: 'Attached' },
  ];
  const OPTIONS = [
    { value: '1d', label: '24 hours' },
    { value: '7d', label: '7 days' },
  ];

  let range = $state('1d');

  const history = liveQuery(
    () => unwrap(api.GET('/v1/usage/history', { params: { query: { window: range as Window } } })),
    { kinds: ['instance.*'] },
  );

  const label = (ms: number) =>
    range === '1d'
      ? new Date(ms).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })
      : new Date(ms).toLocaleString(undefined, { weekday: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });

  const axisLabel = (ms: number) =>
    range === '1d'
      ? new Date(ms).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })
      : new Date(ms).toLocaleDateString(undefined, { weekday: 'short', day: 'numeric' });

  const points = $derived(
    (history.data?.buckets ?? []).map((b) => ({
      x: label(b.start_ms),
      label: axisLabel(b.start_ms),
      values: { chromium: b.chromium, camoufox: b.camoufox, attached: b.attached },
    })),
  );
  const idle = $derived(points.every((p) => Object.values(p.values).every((v) => v === 0)));
</script>

<Card title="Browser usage" description="Browsers running over time">
  {#snippet actions()}
    <SegmentedControl label="Time range" options={OPTIONS} bind:value={range} />
  {/snippet}
  {#if history.error && !history.data}
    <EmptyState tone="danger" compact title="Couldn't load usage." description={history.error.message}>
      {#snippet action()}<Button size="sm" onclick={() => history.reload()}>Retry</Button>{/snippet}
    </EmptyState>
  {:else if history.loading}
    <Skeleton height="200px" radius="var(--radius-md)" />
  {:else if idle}
    <EmptyState compact title="No browsers ran in this period." />
  {:else}
    <AreaChart label="Browsers running over time" series={SERIES} data={points} format={(n) => (Number.isInteger(n) ? String(n) : n.toFixed(1))} />
  {/if}
</Card>
