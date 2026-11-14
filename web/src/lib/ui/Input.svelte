<script lang="ts">
  import type { HTMLInputAttributes } from 'svelte/elements';
  import Badge from './Badge.svelte';

  let {
    value = $bindable(),
    label,
    hideLabel = false,
    id,
    type = 'text',
    placeholder,
    hint,
    error = null,
    required,
    disabled,
    readonly,
    autocomplete,
    list,
    min,
    max,
    step,
    inputmode,
    badge,
  }: {
    value?: string | number | null;
    label: string;
    hideLabel?: boolean;
    id?: string;
    type?: 'text' | 'password' | 'number' | 'url' | 'search' | 'time' | 'email';
    placeholder?: string;
    hint?: string;
    error?: string | null;
    required?: boolean;
    disabled?: boolean;
    readonly?: boolean;
    autocomplete?: HTMLInputAttributes['autocomplete'];
    list?: string;
    min?: number | string;
    max?: number | string;
    step?: number | string;
    badge?: string;
    inputmode?: 'none' | 'text' | 'tel' | 'url' | 'email' | 'numeric' | 'decimal' | 'search';
  } = $props();

  const uid = $props.id();
  const inputId = $derived(id ?? `input-${uid}`);
  const describedBy = $derived(
    [error ? `${inputId}-error` : null, hint ? `${inputId}-hint` : null].filter(Boolean).join(' ') || undefined,
  );
</script>

<div class="field">
  {#if badge}
    <div class="label-row">
      <label class={hideLabel ? 'sr-only' : 'field-label'} for={inputId}>{label}</label>
      <Badge>{badge}</Badge>
    </div>
  {:else}
    <label class={hideLabel ? 'sr-only' : 'field-label'} for={inputId}>{label}</label>
  {/if}
  <input
    class="control"
    id={inputId}
    {type}
    bind:value
    {placeholder}
    {required}
    {disabled}
    {readonly}
    {autocomplete}
    {list}
    {min}
    {max}
    {step}
    {inputmode}
    aria-invalid={error ? 'true' : undefined}
    aria-describedby={describedBy}
  />
  {#if error || hint}
    <div class="field-note">
      {#if error}<p class="field-error" id="{inputId}-error">{error}</p>{/if}
      {#if hint}<p class="field-hint" id="{inputId}-hint">{hint}</p>{/if}
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
</style>
