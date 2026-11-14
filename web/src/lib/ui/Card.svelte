<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title,
    description,
    actions,
    id,
    flush = false,
    children,
  }: { title?: string; description?: string; actions?: Snippet; id?: string; flush?: boolean; children: Snippet } = $props();
</script>

<section class="card" {id} aria-labelledby={title && id ? `${id}-title` : undefined}>
  {#if title}
    <header>
      <div>
        <h2 id={id ? `${id}-title` : undefined} tabindex="-1">{title}</h2>
        {#if description}<p>{description}</p>{/if}
      </div>
      {#if actions}<div class="actions">{@render actions()}</div>{/if}
    </header>
  {/if}
  <div class="body" class:flush>{@render children()}</div>
</section>

<style>
  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4) var(--space-5);
    border-bottom: 1px solid var(--border);
  }

  h2 {
    font-size: var(--text-md-size);
    line-height: var(--text-md-line);
    letter-spacing: 0;
  }

  p {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .actions {
    display: flex;
    gap: var(--space-2);
  }

  .body {
    padding: var(--space-5);
  }

  .body.flush {
    padding: 0;
  }
</style>
