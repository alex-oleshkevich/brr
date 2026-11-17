<script lang="ts">
  type Option = { value: string; label: string; hint?: string; disabled?: boolean };

  let {
    value = $bindable(),
    name,
    label,
    options,
    disabled = false,
  }: { value?: string; name?: string; label: string; options: Option[]; disabled?: boolean } = $props();

  let group: HTMLElement;

  const selected = $derived(options.find((o) => o.value === value));
  const enabled = $derived(options.flatMap((o, i) => (o.disabled ? [] : [i])));
  // A disabled option stays reachable by Tab when it is the saved value, so the group is never unfocusable.
  const tabStop = $derived(selected ? options.indexOf(selected) : (enabled[0] ?? 0));

  function choose(option: Option) {
    if (!option.disabled) value = option.value;
  }

  // Arrow keys move among enabled options only, wrapping at either end.
  function onkeydown(e: KeyboardEvent, index: number) {
    if (enabled.length === 0) return;
    const after = enabled.find((i) => i > index) ?? enabled[0];
    const before = enabled.findLast((i) => i < index) ?? enabled[enabled.length - 1];
    const target = { ArrowRight: after, ArrowDown: after, ArrowLeft: before, ArrowUp: before, Home: enabled[0], End: enabled[enabled.length - 1] }[e.key];
    if (target === undefined) return;

    e.preventDefault();
    value = options[target].value;
    group.querySelectorAll<HTMLElement>('[role="radio"]')[target].focus();
  }
</script>

<div class="segmented">
  <fieldset class="track" role="radiogroup" {name} aria-label={label} bind:this={group}>
    {#each options as option, index (option.value)}
      <button
        type="button"
        role="radio"
        class="item"
        aria-checked={option.value === value}
        aria-disabled={option.disabled || undefined}
        title={option.disabled ? option.hint : undefined}
        tabindex={index === tabStop ? 0 : -1}
        {disabled}
        onclick={() => choose(option)}
        onkeydown={(e) => onkeydown(e, index)}
      >
        {option.label}
      </button>
    {/each}
  </fieldset>
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
    min-inline-size: 0;
    margin: 0;
    padding: 2px;
    background: var(--bg-muted);
    border: 0;
    border-radius: var(--radius-md);
  }

  .item {
    /* A fieldset's flex items don't stretch to its fixed height, so each item takes it directly. */
    height: calc(var(--control-h-md) - 4px);
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

  .item:disabled,
  .item[aria-disabled='true'] {
    cursor: not-allowed;
  }

  .item:disabled:not([aria-checked='true']),
  .item[aria-disabled='true']:not([aria-checked='true']) {
    opacity: 0.5;
  }

  .item:hover:not(:disabled, [aria-disabled='true']) {
    color: var(--text);
  }

  .item[aria-checked='true'] {
    background: var(--segment-selected-bg);
    box-shadow: var(--shadow-sm), inset 0 0 0 1px var(--segment-selected-border);
    color: var(--text);
    font-weight: 500;
  }
</style>
