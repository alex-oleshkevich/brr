<script lang="ts">
  let {
    checked = $bindable(false),
    name,
    label,
    hint,
    disabled,
    onchange,
  }: { checked?: boolean; name?: string; label: string; hint?: string; disabled?: boolean; onchange?: (checked: boolean) => void } = $props();

  const uid = $props.id();
</script>

<div class="checkbox">
  <input id="check-{uid}" type="checkbox" {name} bind:checked {disabled} onchange={(e) => onchange?.(e.currentTarget.checked)} aria-describedby={hint ? `check-${uid}-hint` : undefined} />
  <label for="check-{uid}">{label}</label>
  {#if hint}<p class="field-hint" id="check-{uid}-hint">{hint}</p>{/if}
</div>

<style>
  .checkbox {
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: var(--space-2);
    align-items: center;
  }

  input {
    width: 16px;
    height: 16px;
    margin: 0;
    accent-color: var(--accent);
    cursor: pointer;
  }

  label {
    cursor: pointer;
  }

  input:disabled,
  input:disabled + label {
    color: var(--text-disabled);
    cursor: not-allowed;
  }

  .field-hint {
    grid-column: 2;
  }
</style>
