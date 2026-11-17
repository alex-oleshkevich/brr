<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { capacity } from '$lib/capacity.svelte';
  import LaunchDialog from '$lib/instances/LaunchDialog.svelte';
  import { overviewData } from '$lib/overview/data.svelte';
  import GetStarted from '$lib/overview/GetStarted.svelte';
  import NeedsYou from '$lib/overview/NeedsYou.svelte';
  import NoBrowser from '$lib/overview/NoBrowser.svelte';
  import SpendCard from '$lib/overview/SpendCard.svelte';
  import StatStrip from '$lib/overview/StatStrip.svelte';
  import TrafficCard from '$lib/overview/TrafficCard.svelte';
  import UsageCard from '$lib/overview/UsageCard.svelte';
  import RunTable from '$lib/runs/RunTable.svelte';
  import { FINAL_STATES, isFinal } from '$lib/runs/runs';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';

  const LIMIT = 5;

  const data = overviewData();
  const recent = liveQuery(async () => (await unwrap(api.GET('/v1/instances', { params: { query: { state: FINAL_STATES, limit: LIMIT } } }))).instances, {
    kinds: ['instance.*'],
  });

  let launchOpen = $state(false);

  const hasBrowser = $derived(data.builds.data?.some((b) => ['installed', 'registered'].includes(b.state)) ?? false);
  const running = $derived(
    (capacity.usage?.instances ?? [])
      .filter((i) => !isFinal(i))
      .toSorted((a, b) => (b.started_at ?? b.queued_at) - (a.started_at ?? a.queued_at))
      .slice(0, LIMIT),
  );
</script>

<PageHeader title="Overview">
  {#snippet actions()}
    {#if hasBrowser}<Button variant="primary" icon="play" onclick={() => (launchOpen = true)}>Launch</Button>{/if}
  {/snippet}
</PageHeader>

{#if data.builds.loading}
  <div class="stack" aria-busy="true">
    <Skeleton height="96px" radius="var(--radius-lg)" />
    <Skeleton height="200px" radius="var(--radius-lg)" />
  </div>
{:else if data.builds.error && !data.builds.data}
  <Card>
    <EmptyState tone="danger" title="Couldn't load browser versions" description={data.builds.error.message}>
      {#snippet action()}<Button onclick={() => data.builds.reload()}>Retry</Button>{/snippet}
    </EmptyState>
  </Card>
{:else if !hasBrowser}
  <div class="stack">
    <GetStarted {data} onlaunch={() => (launchOpen = true)} />
    <NoBrowser builds={data.builds.data ?? []} onchange={() => data.builds.reload()} />
  </div>
{:else}
  <div class="stack">
    <GetStarted {data} onlaunch={() => (launchOpen = true)} />
    <NeedsYou {data} />
    <StatStrip {data} />

    <div class="pair">
      <UsageCard />
      <div class="column">
        <SpendCard {data} />
        <TrafficCard {data} />
      </div>
    </div>

    <Card title="Running now" flush>
      {#snippet actions()}<Button variant="ghost" size="sm" href="/browsers">All browsers</Button>{/snippet}
      <RunTable compact runs={running} loading={capacity.loading} error={capacity.error} onretry={() => capacity.reload()} />
    </Card>

    <Card title="Recent runs" flush>
      {#snippet actions()}<Button variant="ghost" size="sm" href="/history">All history</Button>{/snippet}
      <RunTable compact runs={recent.data} loading={recent.loading} error={recent.error} onretry={() => recent.reload()} />
    </Card>
  </div>
{/if}

<LaunchDialog bind:open={launchOpen} />

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .pair {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-4);
    align-items: start;
  }

  .column {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  @media (max-width: 899px) {
    .pair {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
