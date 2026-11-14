<script lang="ts">
  import type { PagedQuery } from '$lib/api/paged.svelte';
  import { count } from '$lib/format';
  import Button from './Button.svelte';

  let { list }: { list: PagedQuery<unknown> } = $props();
</script>

{#if list.capped}
  <p>Showing the first {count(list.max)}. Narrow the filters to see more.</p>
{:else}
  <Button variant="ghost" loading={list.loadingMore} onclick={list.loadMore}>Load more</Button>
{/if}

<style>
  p {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
  }
</style>
