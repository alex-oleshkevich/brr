<script lang="ts">
  import type { BrrEvent } from '$lib/api/events.svelte';
  import { ApiError } from '$lib/api/client';
  import { clock } from '$lib/clock.svelte';
  import { kindLabel } from '$lib/events/kinds';
  import { summarize } from '$lib/events/summary';
  import { relative } from '$lib/format';
  import { stateTone, type Tone } from '$lib/state';
  import Button from '$lib/ui/Button.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';

  let {
    events,
    loading,
    error,
    onretry,
  }: { events: BrrEvent[] | undefined; loading: boolean; error: ApiError | null; onretry: () => void } = $props();

  function tone(e: BrrEvent): Tone {
    const data = e.data as { to?: string } | null;
    if (e.kind === 'instance.state_changed' && data?.to) return stateTone('instance', data.to);
    if (/crashed|failed/.test(e.kind)) return 'danger';
    if (/awaiting|warning/.test(e.kind)) return 'warning';
    return 'neutral';
  }

  const solid = (t: Tone) => (t === 'accent' ? 'var(--accent)' : `var(--${t}-solid)`);
</script>

{#if loading}
  <ul aria-busy="true">
    {#each { length: 5 } as _, i (i)}
      <li><Skeleton width="100%" /></li>
    {/each}
  </ul>
{:else if error}
  <EmptyState tone="danger" title="Couldn't load activity" description={error.message}>
    {#snippet action()}<Button onclick={onretry}>Retry</Button>{/snippet}
  </EmptyState>
{:else if !events || events.length === 0}
  <EmptyState icon="activity" title="No activity yet" />
{:else}
  <!-- The list scrolls, so keyboard users need to reach it. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <ul tabindex="0" aria-label="Activity">
    {#each events as e (e.seq)}
      <li>
        <span class="dot" style:background={solid(tone(e))}></span>
        <div class="text">
          <p class="headline truncate" title={summarize(e)}>{summarize(e) || kindLabel(e.kind)}</p>
          <p class="kind">{kindLabel(e.kind)}</p>
        </div>
        <time class="time" datetime={new Date(e.ts).toISOString()}>{relative(e.ts, clock.now)}</time>
      </li>
    {/each}
  </ul>
{/if}

<style>
  ul {
    display: flex;
    flex-direction: column;
    max-height: 420px;
    overflow-y: auto;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: grid;
    grid-template-columns: 6px minmax(0, 1fr) auto;
    gap: var(--space-3);
    align-items: start;
    padding: var(--space-2) 0;
  }

  .dot {
    width: 6px;
    height: 6px;
    margin-top: var(--space-2);
    border-radius: var(--radius-full);
  }

  .headline {
    font-weight: 500;
  }

  .kind,
  .time {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .time {
    color: var(--text-tertiary);
    white-space: nowrap;
  }
</style>
