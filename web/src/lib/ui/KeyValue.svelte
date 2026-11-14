<script lang="ts">
  import { toast } from './toast.svelte';
  import Button from './Button.svelte';

  type Item = { label: string; value: string | number | null; mono?: boolean; copy?: boolean; href?: string; tone?: 'danger' };

  let { items, grid = false }: { items: Item[]; grid?: boolean } = $props();

  const canCopy = typeof navigator !== 'undefined' && !!navigator.clipboard;

  async function copy(item: Item) {
    await navigator.clipboard.writeText(String(item.value));
    toast.success('Copied');
  }
</script>

<dl class:grid>
  {#each items as item (item.label)}
    <div class="row">
      <dt>{item.label}</dt>
      <dd>
        {#if item.value === null}
          <span class="none">—</span>
        {:else if item.href}
          <a class:mono={item.mono} href={item.href}>{item.value}</a>
        {:else}
          <span class:mono={item.mono} class:danger={item.tone === 'danger'}>{item.value}</span>
        {/if}
        {#if item.copy && canCopy && item.value !== null}
          <Button variant="ghost" size="sm" iconOnly icon="copy" aria-label="Copy {item.label}" onclick={() => copy(item)} />
        {/if}
      </dd>
    </div>
  {/each}
</dl>

<style>
  dl {
    margin: 0;
  }

  .row {
    display: grid;
    grid-template-columns: 140px minmax(0, 1fr);
    gap: var(--space-4);
    align-items: center;
    min-height: var(--space-12);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--border);
  }

  .row:last-child {
    border-bottom: 0;
  }

  /* Label above value, as many columns as fit: for summaries with many short values. */
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--space-4) var(--space-6);
  }

  .grid dd {
    min-height: var(--control-h-sm);
  }

  .grid .row {
    grid-template-columns: minmax(0, 1fr);
    gap: var(--space-1);
    align-items: start;
    min-height: 0;
    padding: 0;
    border-bottom: 0;
  }

  dt {
    color: var(--text-secondary);
  }

  dd {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    margin: 0;
    overflow-wrap: anywhere;
  }

  .none {
    color: var(--text-tertiary);
  }

  .danger {
    color: var(--danger-fg);
  }

  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-sm-size);
  }
</style>
