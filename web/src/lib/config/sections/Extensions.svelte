<script lang="ts">
  import Switch from '$lib/ui/Switch.svelte';
  import ExtensionPicker from '$lib/extensions/ExtensionPicker.svelte';
  import { engineOf, type SectionProps } from '../form';

  let { settings, readonly = false, errors, builds, library, profileExtensions, onExtensions, extUnsupported }: SectionProps = $props();

  const engine = $derived(engineOf(settings, builds));
</script>

<div class="fields">
  <Switch label="brr companion extension" bind:checked={settings.companion} disabled={readonly} hint="Lets brr control tabs in this browser through the extension." />
  <ExtensionPicker {library} {engine} value={profileExtensions} onchange={onExtensions} {readonly} />
  {#if extUnsupported}
    <p class="danger" role="alert">This browser version can't load extensions.</p>
  {:else if errors['extensions'] || errors['settings.extensions']}
    <p class="danger" role="alert">{errors['extensions'] ?? errors['settings.extensions']}</p>
  {/if}
</div>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .danger {
    padding: var(--space-3);
    background: var(--danger-bg);
    border: 1px solid var(--danger-border);
    border-radius: var(--radius-md);
    color: var(--danger-fg);
  }
</style>
