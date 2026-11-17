<script lang="ts">
  import Input from '$lib/ui/Input.svelte';
  import Textarea from '$lib/ui/Textarea.svelte';

  let {
    name = $bindable(''),
    tags = $bindable(''),
    notes = $bindable(''),
    showMeta = true,
    readonly = false,
    nameError = null,
    tagsError = null,
  }: {
    name?: string;
    tags?: string;
    notes?: string;
    showMeta?: boolean;
    readonly?: boolean;
    nameError?: string | null;
    tagsError?: string | null;
  } = $props();
</script>

<div class="fields">
  <Input label="Name" bind:value={name} readonly={readonly} error={nameError} required />
  {#if showMeta}
    <Input label="Tags" bind:value={tags} readonly={readonly} hint="Comma-separated. Used for per-tag limits" error={tagsError} />
    <Textarea label="Notes" bind:value={notes} readonly={readonly} rows={3} />
  {/if}
</div>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
</style>
