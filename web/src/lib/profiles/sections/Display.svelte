<script lang="ts">
  import { untrack } from 'svelte';
  import Input from '$lib/ui/Input.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import { halfPair, type SectionProps } from '../form';

  let { settings, readonly = false, reportInvalid }: SectionProps = $props();

  let display = $state(untrack(() => (settings.display)));
  let width = $state<number | null>(untrack(() => (settings.screen?.width ?? null)));
  let height = $state<number | null>(untrack(() => (settings.screen?.height ?? null)));

  $effect(() => {
    settings.display = display;
  });
  const half = $derived(halfPair(width, height));

  $effect(() => {
    reportInvalid('screen', half.first || half.second);
    return () => reportInvalid('screen', false);
  });
  $effect(() => {
    settings.screen = width !== null && height !== null ? { width, height } : null;
  });
</script>

<div class="fields">
  <div class="field">
    <span class="field-label">Display mode</span>
    <SegmentedControl
      disabled={readonly}
      label="Display mode"
      bind:value={display}
      options={[
        { value: 'headless', label: 'Headless' },
        { value: 'visible', label: 'Visible', hint: 'Opens a desktop window. Needs DISPLAY or WAYLAND_DISPLAY.' },
        { value: 'virtual', label: 'Virtual', hint: 'Runs on a hidden display sized to the screen below. Needs Xvfb installed.' },
      ]}
    />
  </div>
  <div class="pair">
    <Input label="Screen width" type="number" min={0} bind:value={width} readonly={readonly} error={half.first ? 'Enter a width too, or clear the height.' : null} />
    <Input label="Screen height" type="number" min={0} bind:value={height} readonly={readonly} error={half.second ? 'Enter a height too, or clear the width.' : null} />
  </div>
</div>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }
</style>
