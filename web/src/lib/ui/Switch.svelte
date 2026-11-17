<script lang="ts">
  let {
    checked = $bindable(false),
    name,
    label,
    hint,
    disabled,
    hideLabel = false,
    onchange,
  }: { checked?: boolean; name?: string; label: string; hint?: string; disabled?: boolean; hideLabel?: boolean; onchange?: (checked: boolean) => void } = $props();

  const uid = $props.id();
</script>

<div class="switch">
  <button
    type="button"
    role="switch"
    id="switch-{uid}"
    {name}
    class="track"
    aria-checked={checked}
    aria-labelledby="switch-{uid}-label"
    aria-describedby={hint ? `switch-${uid}-hint` : undefined}
    {disabled}
    onclick={() => {
      checked = !checked;
      onchange?.(checked);
    }}
  >
    <span class="thumb"></span>
  </button>
  <label class={hideLabel ? 'sr-only' : 'label'} id="switch-{uid}-label" for="switch-{uid}">{label}</label>
  {#if hint}<p class="field-hint" id="switch-{uid}-hint">{hint}</p>{/if}
</div>

<style>
  .switch {
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: var(--space-2);
    align-items: center;
  }

  .track {
    position: relative;
    width: 32px;
    height: 18px;
    padding: 0;
    background: var(--bg-emphasis);
    border: 1px solid var(--border-control);
    border-radius: var(--radius-full);
    cursor: pointer;
    transition:
      background-color var(--duration-fast) var(--ease-standard),
      border-color var(--duration-fast) var(--ease-standard);
  }

  .track[aria-checked='true'] {
    background: var(--accent);
    border-color: var(--accent);
  }

  .thumb {
    position: absolute;
    top: 1px;
    left: 1px;
    width: 14px;
    height: 14px;
    background: #fff;
    border-radius: var(--radius-full);
    box-shadow: var(--shadow-sm);
    transition: transform var(--duration-fast) var(--ease-standard);
  }

  .track[aria-checked='true'] .thumb {
    transform: translateX(14px);
  }

  .track:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .label {
    cursor: pointer;
  }

  .switch:has(.track:disabled) .label {
    cursor: not-allowed;
  }

  .field-hint {
    grid-column: 2;
  }
</style>
