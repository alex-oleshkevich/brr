<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title,
    description,
    breadcrumb,
    actions,
    badge,
  }: {
    title: string;
    description?: string;
    breadcrumb?: { label: string; href: string }[];
    actions?: Snippet;
    badge?: Snippet;
  } = $props();
</script>

<svelte:head><title>{title} · brr</title></svelte:head>

<header>
  <div class="text">
    {#if breadcrumb?.length}
      <nav aria-label="Breadcrumb">
        {#each breadcrumb as crumb, index}
          {#if index > 0}<span aria-hidden="true">/</span>{/if}
          <a href={crumb.href}>{crumb.label}</a>
        {/each}
      </nav>
    {/if}
    <div class="title">
      <h1>{title}</h1>
      {@render badge?.()}
    </div>
    {#if description}<p>{description}</p>{/if}
  </div>
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</header>

<style>
  header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--space-4);
    margin-bottom: var(--space-6);
  }

  nav {
    display: flex;
    gap: var(--space-2);
    margin-bottom: var(--space-1);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    color: var(--text-secondary);
  }

  nav a {
    color: var(--text-secondary);
  }

  nav a:hover {
    color: var(--text);
  }

  .title {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  h1 {
    font-size: var(--text-xl-size);
    line-height: var(--text-xl-line);
    letter-spacing: -0.005em;
  }

  p {
    margin-top: var(--space-1);
    color: var(--text-secondary);
  }

  .actions {
    align-self: flex-start;
    display: flex;
    gap: var(--space-2);
  }
</style>
