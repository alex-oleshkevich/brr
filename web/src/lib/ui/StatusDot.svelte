<script lang="ts">
  import { events } from '$lib/api/events.svelte';
  import type { Tone } from '$lib/state';

  let {
    tone,
    label,
    showLabel = true,
    pulse = false,
  }: { tone: Tone; label: string; showLabel?: boolean; pulse?: boolean } = $props();

  // While the daemon is unreachable a green or blue dot is the last thing it said, not the current state.
  const stale = $derived(events.status === 'reconnecting' && (tone === 'success' || tone === 'accent'));
  const solid = $derived(tone === 'accent' ? 'var(--accent)' : `var(--${tone}-solid)`);
</script>

<span class="status">
  <span class="dot" class:stale class:status-dot-pulse={pulse && !stale} style:background={solid} style:--pulse={solid}></span>
  <span class={showLabel ? 'label' : 'sr-only'}>{label}</span>
</span>

<style>
  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
  }

  .label {
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    color: var(--text-secondary);
  }

  .stale {
    opacity: 0.35;
  }

  .status-dot-pulse {
    animation: pulse 2s ease-out infinite;
  }

  @keyframes pulse {
    0% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--pulse) 50%, transparent);
    }
    70%,
    100% {
      box-shadow: 0 0 0 6px transparent;
    }
  }
</style>
