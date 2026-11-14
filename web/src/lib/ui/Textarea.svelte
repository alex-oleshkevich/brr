<script lang="ts">
  let {
    value = $bindable(),
    label,
    hideLabel = false,
    id,
    placeholder,
    hint,
    error = null,
    required,
    disabled,
    readonly,
    rows = 4,
  }: {
    value?: string;
    label: string;
    hideLabel?: boolean;
    id?: string;
    placeholder?: string;
    hint?: string;
    error?: string | null;
    required?: boolean;
    disabled?: boolean;
    readonly?: boolean;
    rows?: number;
  } = $props();

  const uid = $props.id();
  const inputId = $derived(id ?? `textarea-${uid}`);
  const describedBy = $derived(
    [error ? `${inputId}-error` : null, hint ? `${inputId}-hint` : null].filter(Boolean).join(' ') || undefined,
  );
</script>

<div class="field">
  <label class={hideLabel ? 'sr-only' : 'field-label'} for={inputId}>{label}</label>
  <textarea
    class="control"
    id={inputId}
    bind:value
    {rows}
    {placeholder}
    {required}
    {disabled}
    {readonly}
    aria-invalid={error ? 'true' : undefined}
    aria-describedby={describedBy}
  ></textarea>
  {#if error || hint}
    <div class="field-note">
      {#if error}<p class="field-error" id="{inputId}-error">{error}</p>{/if}
      {#if hint}<p class="field-hint" id="{inputId}-hint">{hint}</p>{/if}
    </div>
  {/if}
</div>
