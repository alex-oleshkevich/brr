<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { ApiError } from '$lib/api/client';
  import Button from '$lib/ui/Button.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';

  let { loading, error, onretry, title, children }: { loading: boolean; error: ApiError | null; onretry: () => void; title: string; children: Snippet } = $props();
</script>

{#if loading}
  <div class="skeletons" aria-busy="true">
    {#each { length: 4 } as _, i (i)}<Skeleton width="{50 + ((i * 17) % 40)}%" />{/each}
  </div>
{:else if error}
  <EmptyState tone="danger" title="Couldn't load {title}" description={error.message}>
    {#snippet action()}<Button onclick={onretry}>Retry</Button>{/snippet}
  </EmptyState>
{:else}
  {@render children()}
{/if}

<style>
  .skeletons {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
</style>
