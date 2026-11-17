<script lang="ts">
  import { untrack } from 'svelte';
  import Input from '$lib/ui/Input.svelte';
  import Switch from '$lib/ui/Switch.svelte';
  import { fieldError } from '../form';
  import type { SectionProps } from '../form';

  let { settings, readonly = false, errors, defaults, isTemplate, pinned, onPinned }: SectionProps = $props();

  let tabMin = $state<number | null>(untrack(() => (settings.idle.tab_min ?? null)));
  let instanceMin = $state<number | null>(untrack(() => (settings.idle.instance_min ?? null)));
  let pinnedValue = $state(untrack(() => (pinned)));

  $effect(() => {
    const idle = untrack(() => settings.idle);
    settings.idle = { ...idle, tab_min: tabMin, instance_min: instanceMin };
  });
  $effect(() => {
    onPinned(pinnedValue);
  });
</script>

<div class="fields">
  <div class="pair">
    <Input label="Tab idle timeout (min)" type="number" min={1} error={fieldError(errors, 'settings.idle.tab_min', 'idle.tab_min', 'tab_min')} bind:value={tabMin} readonly={readonly} placeholder={readonly ? 'Not set' : defaults ? `Default (${defaults.idle.tab_min})` : undefined} />
    <Input label="Instance idle timeout (min)" type="number" min={1} error={fieldError(errors, 'settings.idle.instance_min', 'idle.instance_min', 'instance_min')} bind:value={instanceMin} readonly={readonly} placeholder={readonly ? 'Not set' : defaults ? `Default (${defaults.idle.instance_min})` : undefined} />
  </div>
  {#if !isTemplate}
    <Switch label="Pinned" bind:checked={pinnedValue} disabled={readonly} hint="Never reap or evict this profile's Instances." />
  {/if}
</div>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }
</style>
