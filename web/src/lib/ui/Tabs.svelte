<script lang="ts">
  import Badge from './Badge.svelte';

  let {
    items,
    value = $bindable(),
    label,
    idPrefix,
  }: {
    items: { id: string; label: string; count?: number | string }[];
    value?: string;
    label: string;
    idPrefix: string;
  } = $props();

  let list: HTMLDivElement;

  function onkeydown(e: KeyboardEvent, index: number) {
    const last = items.length - 1;
    const target = { ArrowRight: index === last ? 0 : index + 1, ArrowLeft: index === 0 ? last : index - 1, Home: 0, End: last }[e.key];
    if (target === undefined) return;

    e.preventDefault();
    value = items[target].id;
    list.querySelectorAll<HTMLElement>('[role="tab"]')[target].focus();
  }
</script>

<div class="tabs" role="tablist" aria-label={label} bind:this={list}>
  {#each items as item, index (item.id)}
    <button
      type="button"
      role="tab"
      id="{idPrefix}-tab-{item.id}"
      aria-selected={item.id === value}
      aria-controls="{idPrefix}-panel-{item.id}"
      tabindex={item.id === value ? 0 : -1}
      onclick={() => (value = item.id)}
      onkeydown={(e) => onkeydown(e, index)}
    >
      {item.label}
      {#if item.count !== undefined}<Badge><span class="num">{item.count}</span></Badge>{/if}
    </button>
  {/each}
</div>

<style>
  .tabs {
    display: flex;
    gap: var(--space-5);
    border-bottom: 1px solid var(--border);
  }

  button {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    height: 36px;
    padding: 0;
    background: none;
    border: 0;
    color: var(--text-secondary);
    font-weight: 500;
    cursor: pointer;
    transition: color var(--duration-fast) var(--ease-standard);
  }

  button:hover {
    color: var(--text);
  }

  button[aria-selected='true'] {
    color: var(--text);
    box-shadow: inset 0 -2px var(--accent);
  }
</style>
