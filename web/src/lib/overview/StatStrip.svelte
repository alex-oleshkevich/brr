<script lang="ts">
  import { capacity } from '$lib/capacity.svelte';
  import { clock } from '$lib/clock.svelte';
  import { bytes, relative, usd } from '$lib/format';
  import { settingsHref } from '$lib/settings/sections';
  import Button from '$lib/ui/Button.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import StatCard from '$lib/ui/StatCard.svelte';
  import { trafficByDay, type OverviewData } from './data.svelte';

  type Tone = 'neutral' | 'success' | 'warning' | 'danger';

  const WARN = 0.75;

  let { data }: { data: OverviewData } = $props();

  const toneFor = (ratio: number): Tone => (ratio >= 1 ? 'danger' : ratio >= WARN ? 'warning' : 'success');
  const mb = (n: number) => bytes(n * 1048576);

  const cards = $derived.by(() => {
    const { info, limits } = capacity;
    if (!info || !limits) return [];

    const adaptive = limits.mode === 'adaptive';
    const browsers = capacity.browsers / Math.max(1, limits.max_instances);
    const tabs = capacity.tabs / Math.max(1, limits.max_tabs_total);
    const waiting = capacity.queue?.length ?? 0;
    const floor = limits.adaptive.min_free_ram_mb;
    const ramRatio = floor / Math.max(1, info.mem_available_mb);
    const loadRatio = info.load_per_core / Math.max(0.01, limits.adaptive.max_load_per_core);

    return [
      { label: 'Browsers running', value: `${capacity.browsers} / ${limits.max_instances}`, tone: toneFor(browsers), href: '/browsers' },
      { label: 'Open tabs', value: String(capacity.tabs), sub: `of ${limits.max_tabs_total} allowed`, tone: toneFor(tabs) },
      { label: 'Queued launches', value: String(waiting), sub: waiting ? 'waiting for room' : 'nothing waiting', tone: (waiting ? 'warning' : 'neutral') as Tone },
      {
        label: 'Free memory',
        value: mb(info.mem_available_mb),
        sub: adaptive ? `brr keeps ${mb(floor)} free` : 'not enforced',
        tone: adaptive ? toneFor(ramRatio) : ('neutral' as Tone),
      },
      {
        label: 'CPU load',
        value: `${info.load_per_core.toFixed(2)} per core`,
        sub: adaptive ? `pauses new browsers above ${limits.adaptive.max_load_per_core}` : 'not enforced',
        tone: adaptive ? toneFor(loadRatio) : ('neutral' as Tone),
      },
    ];
  });

  const spend = $derived.by(() => {
    const view = data.captcha.data;
    if (!view) return null;

    const ratio = view.daily_usd_micros > 0 ? view.spent_today_usd_micros / view.daily_usd_micros : 0;
    return {
      label: 'Captcha spend today',
      value: usd(view.spent_today_usd_micros),
      sub: `of ${usd(view.daily_usd_micros)} a day`,
      tone: (view.spent_today_usd_micros > 0 ? toneFor(ratio) : 'neutral') as Tone,
      href: settingsHref('captcha'),
    };
  });

  const balance = $derived(data.balance.data);
  const keySet = $derived(data.captcha.data?.api_key?.set === true);

  const traffic = $derived.by(() => {
    if (!data.traffic.data) return null;

    return {
      label: 'Proxy traffic (7 days)',
      value: bytes(trafficByDay(data.traffic.data).reduce((sum, day) => sum + day.bytes, 0)),
      sub: 'proxied and sandboxed browsers',
    };
  });
</script>

{#if capacity.error && !capacity.info}
  <EmptyState tone="danger" compact title="Couldn't load capacity." description={capacity.error.message}>
    {#snippet action()}<Button size="sm" onclick={() => capacity.reload()}>Retry</Button>{/snippet}
  </EmptyState>
{:else if capacity.loading}
  <div class="strip" aria-busy="true">
    {#each { length: 5 } as _, i (i)}<Skeleton height="96px" radius="var(--radius-lg)" />{/each}
  </div>
{:else}
  <div class="strip">
    {#each cards as card (card.label)}
      <StatCard {...card} />
    {/each}
    {#if spend}<StatCard {...spend} />{/if}
    {#if traffic}<StatCard {...traffic} />{/if}
    {#if keySet && data.balance.error && !balance}
      <StatCard label="Provider balance" value="Unavailable" sub="Couldn't check · Fix" href={settingsHref('captcha')} tone="warning" />
    {:else if balance}
      <StatCard
        label="Provider balance"
        value={usd(balance.balance_usd_micros)}
        sub="updated {relative(balance.fetched_at, clock.now)}"
        href={settingsHref('captcha')}
      />
    {/if}
  </div>
{/if}

<style>
  .strip {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: var(--space-3);
  }
</style>
