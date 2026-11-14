<script lang="ts">
  import type { Snippet } from 'svelte';
  import { events } from '$lib/api/events.svelte';
  import type { Tone } from '$lib/state';

  let { tone = 'neutral', dot = false, children }: { tone?: Tone; dot?: boolean; children: Snippet } = $props();

  const stale = $derived(events.status === 'reconnecting' && (tone === 'success' || tone === 'accent'));
</script>

<span class="badge badge-{tone}">
  {#if dot}<span class="dot" class:stale aria-hidden="true"></span>{/if}
  {@render children()}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    height: 20px;
    padding: 0 var(--space-2);
    border: 1px solid var(--tone-border);
    border-radius: var(--radius-sm);
    background: var(--tone-bg);
    color: var(--tone-fg);
    font-size: var(--text-xs-size);
    line-height: var(--text-xs-line);
    font-weight: 500;
    letter-spacing: 0.01em;
    white-space: nowrap;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--tone-solid);
  }

  .stale {
    opacity: 0.35;
  }

  .badge-neutral {
    --tone-fg: var(--neutral-fg);
    --tone-bg: var(--neutral-bg);
    --tone-border: var(--neutral-border);
    --tone-solid: var(--neutral-solid);
  }

  .badge-accent {
    --tone-fg: var(--accent-text);
    --tone-bg: var(--accent-subtle);
    --tone-border: var(--accent-border);
    --tone-solid: var(--accent);
  }

  .badge-success {
    --tone-fg: var(--success-fg);
    --tone-bg: var(--success-bg);
    --tone-border: var(--success-border);
    --tone-solid: var(--success-solid);
  }

  .badge-warning {
    --tone-fg: var(--warning-fg);
    --tone-bg: var(--warning-bg);
    --tone-border: var(--warning-border);
    --tone-solid: var(--warning-solid);
  }

  .badge-danger {
    --tone-fg: var(--danger-fg);
    --tone-bg: var(--danger-bg);
    --tone-border: var(--danger-border);
    --tone-solid: var(--danger-solid);
  }
</style>
