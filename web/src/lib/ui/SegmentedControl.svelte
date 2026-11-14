<script lang="ts">
  type Option = { value: string; label: string; hint?: string };

  let {
    value = $bindable(),
    label,
    options,
    disabled = false,
  }: { value?: string; label: string; options: Option[]; disabled?: boolean } = $props();

  let group: HTMLDivElement;

  const selected = $derived(options.find((o) => o.value === value));

  function onkeydown(e: KeyboardEvent, index: number) {
    const last = options.length - 1;
    const target = { ArrowRight: index === last ? 0 : index + 1, ArrowDown: index === last ? 0 : index + 1, ArrowLeft: index === 0 ? last : index - 1, ArrowUp: index === 0 ? last : index - 1, Home: 0, End: last }[e.key];
    if (target === undefined) return;

    e.preventDefault();
    value = options[target].value;
    group.querySelectorAll<HTMLElement>('[role="radio"]')[target].focus();
  }
</script>

<div class="segmented">
  <div class="track" role="radiogroup" aria-label={label} bind:this={group}>
    {#each options as option, index (option.value)}
      <button
        type="button"
        role="radio"
        class="item"
        aria-checked={option.value === value}
        tabindex={option.value === value || (value === undefined && index === 0) ? 0 : -1}
        {disabled}
        onclick={() => (value = option.value)}
        onkeydown={(e) => onkeydown(e, index)}
      >
        {option.label}
      </button>
    {/each}
  </div>
  {#if selected?.hint}<p class="field-hint">{selected.hint}</p>{/if}
</div>

<style>
  .segmented {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    align-items: flex-start;
  }

  .track {
    display: inline-flex;
    height: var(--control-h-md);
    padding: 2px;
    background: var(--bg-muted);
    border-radius: var(--radius-md);
  }

  .item {
    padding: 0 var(--space-3);
    background: transparent;
    border: 0;
    border-radius: calc(var(--radius-md) - 2px);
    color: var(--text-secondary);
    font-weight: 400;
    cursor: pointer;
    transition:
      background-color var(--duration-fast) var(--ease-standard),
      color var(--duration-fast) var(--ease-standard);
  }

  .item:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .item:hover:not(:disabled) {
    color: var(--text);
  }

  .item[aria-checked='true'] {
    background: var(--segment-selected-bg);
    box-shadow: var(--shadow-sm), inset 0 0 0 1px var(--segment-selected-border);
    color: var(--text);
    font-weight: 500;
  }
</style>
