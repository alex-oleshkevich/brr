<script lang="ts">
  import Badge from './Badge.svelte';
  import Icon from './Icon.svelte';

  type Option = { value: string; label: string; disabled?: boolean };

  let {
    value = $bindable(),
    label,
    hideLabel = false,
    options,
    hint,
    error = null,
    disabled,
    badge,
    onchange,
  }: {
    value?: string;
    label: string;
    hideLabel?: boolean;
    options: (Option | { group: string; options: Option[] })[];
    hint?: string;
    error?: string | null;
    disabled?: boolean;
    badge?: string;
    onchange?: (value: string) => void;
  } = $props();

  const uid = $props.id();
  const selectId = `select-${uid}`;
  const describedBy = $derived(
    [error ? `${selectId}-error` : null, hint ? `${selectId}-hint` : null].filter(Boolean).join(' ') || undefined,
  );
</script>

{#snippet item(option: Option)}
  <option value={option.value} disabled={option.disabled}>{option.label}</option>
{/snippet}

<div class="field">
  {#if badge}
    <div class="label-row">
      <label class={hideLabel ? 'sr-only' : 'field-label'} for={selectId}>{label}</label>
      <Badge>{badge}</Badge>
    </div>
  {:else}
    <label class={hideLabel ? 'sr-only' : 'field-label'} for={selectId}>{label}</label>
  {/if}
  <div class="wrap">
    <select
      class="control"
      id={selectId}
      bind:value
      onchange={(e) => onchange?.(e.currentTarget.value)}
      {disabled}
      aria-invalid={error ? 'true' : undefined}
      aria-describedby={describedBy}
    >
      {#each options as option}
        {#if 'group' in option}
          <optgroup label={option.group}>
            {#each option.options as inner}{@render item(inner)}{/each}
          </optgroup>
        {:else}
          {@render item(option)}
        {/if}
      {/each}
    </select>
    <span class="chevron"><Icon name="chevron-down" /></span>
  </div>
  {#if error || hint}
    <div class="field-note">
      {#if error}<p class="field-error" id="{selectId}-error">{error}</p>{/if}
      {#if hint}<p class="field-hint" id="{selectId}-hint">{hint}</p>{/if}
    </div>
  {/if}
</div>

<style>
  .label-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-2);
  }

  .label-row label {
    white-space: nowrap;
  }

  .wrap {
    position: relative;
  }

  select {
    appearance: none;
    padding-right: var(--space-8);
    cursor: pointer;
  }

  .chevron {
    position: absolute;
    top: 50%;
    right: var(--space-3);
    display: flex;
    transform: translateY(-50%);
    color: var(--text-secondary);
    pointer-events: none;
  }
</style>
