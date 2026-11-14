<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from './Icon.svelte';

  let {
    title,
    description,
    tone = 'neutral',
    icon,
    level = 2,
    compact = false,
    action,
  }: {
    title: string;
    description?: string;
    tone?: 'neutral' | 'danger';
    icon?: IconName;
    // A page-level state (404, error) is the page's only heading.
    level?: 1 | 2;
    // One quiet line for a card that is only empty, not a page-level state.
    compact?: boolean;
    action?: Snippet;
  } = $props();
</script>

{#if compact}
  <p class="compact">{title}{#if description}<span> {description}</span>{/if}</p>
{:else}
<div class="empty">
  <span class="badge badge-{tone}"><Icon name={icon ?? (tone === 'danger' ? 'alert' : 'info')} size={20} /></span>
  <svelte:element this={`h${level}`}>{title}</svelte:element>
  {#if description}<p>{description}</p>{/if}
  {#if action}<div class="action">{@render action()}</div>{/if}
</div>
{/if}

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: var(--space-12) var(--space-6);
    text-align: center;
  }

  .badge {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: var(--radius-full);
    background: var(--bg-muted);
    color: var(--text-secondary);
  }

  .badge-danger {
    background: var(--danger-bg);
    color: var(--danger-fg);
  }

  h1,
  h2 {
    margin-top: var(--space-3);
    font-size: var(--text-md-size);
    line-height: var(--text-md-line);
    letter-spacing: 0;
  }

  p {
    max-width: 360px;
    margin-top: var(--space-1);
    color: var(--text-secondary);
  }

  p.compact {
    max-width: none;
    margin: 0;
    padding: var(--space-3) var(--space-4);
    color: var(--text-secondary);
  }

  .action {
    margin-top: var(--space-4);
  }
</style>
