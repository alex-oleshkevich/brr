<script lang="ts">
  import Button from './Button.svelte';

  let {
    dirty,
    saving = false,
    disabled = false,
    onsave,
    ondiscard,
  }: { dirty: boolean; saving?: boolean; disabled?: boolean; onsave: () => void; ondiscard: () => void } = $props();
</script>

{#if dirty}
  <div class="savebar" role="region" aria-label="Unsaved changes">
    <span>Unsaved changes</span>
    <span class="actions">
      <Button variant="ghost" size="sm" onclick={ondiscard} disabled={saving}>Discard</Button>
      <Button variant="primary" size="sm" onclick={onsave} loading={saving} {disabled}>Save</Button>
    </span>
  </div>
{/if}

<style>
  .savebar {
    position: sticky;
    bottom: var(--space-4);
    z-index: 5;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin-top: var(--space-4);
    padding: var(--space-2) var(--space-3);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-md);
  }

  .actions {
    display: flex;
    gap: var(--space-2);
  }
</style>
