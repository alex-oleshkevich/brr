<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let { summary, open = $bindable(false), children }: { summary: string; open?: boolean; children: Snippet } = $props();
</script>

<details bind:open>
  <summary><Icon name="chevron-right" />{summary}</summary>
  <div class="content">{@render children()}</div>
</details>

<style>
  details {
    border-top: 1px solid var(--border);
    padding-top: var(--space-3);
  }

  summary {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--text-secondary);
    font-weight: 500;
    cursor: pointer;
    list-style: none;
  }

  summary::-webkit-details-marker {
    display: none;
  }

  summary:hover {
    color: var(--text);
  }

  summary :global(svg) {
    transition: transform var(--duration-fast) var(--ease-standard);
  }

  details[open] > summary :global(svg) {
    transform: rotate(90deg);
  }

  .content {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    margin-top: var(--space-4);
  }
</style>
