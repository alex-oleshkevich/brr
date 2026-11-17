<script lang="ts">
  import { untrack } from 'svelte';
  import { capacity } from '$lib/capacity.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import { halfPair, type SectionProps } from '../form';

  const CHOICES = [
    { value: 'headless', label: 'Headless' },
    { value: 'visible', label: 'Visible', hint: 'Opens a desktop window. Needs DISPLAY or WAYLAND_DISPLAY.' },
    { value: 'virtual', label: 'Virtual', hint: 'Runs on a hidden display sized to the screen below. Needs Xvfb installed.' },
  ];

  let { settings, readonly = false, reportInvalid }: SectionProps = $props();

  let display = $state(untrack(() => (settings.display)));
  let width = $state<number | null>(untrack(() => (settings.screen?.width ?? null)));
  let height = $state<number | null>(untrack(() => (settings.screen?.height ?? null)));

  $effect(() => {
    settings.display = display;
  });
  const half = $derived(halfPair(width, height));
  const widthError = $derived(width !== null && width < 0 ? 'Enter 0 or more.' : half.first ? 'Enter a width too, or clear the height.' : null);
  const heightError = $derived(height !== null && height < 0 ? 'Enter 0 or more.' : half.second ? 'Enter a height too, or clear the width.' : null);
  // The warning below already gives the saved choice's reason, so its hint would only repeat it.
  const choices = $derived(
    CHOICES.map((c) => {
      const reason = capacity.displayUnavailable.get(c.value);
      return reason ? { value: c.value, label: c.label, hint: c.value === display ? undefined : reason, disabled: true } : c;
    }),
  );
  const savedChoice = $derived(CHOICES.find((c) => c.value === display));
  const savedReason = $derived(capacity.displayUnavailable.get(display));

  $effect(() => {
    reportInvalid('screen', widthError !== null || heightError !== null);
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
      options={choices}
    />
  </div>
  {#if savedChoice && savedReason}
    <p class="warning" role="status">{savedChoice.label} can't start here: {savedReason}. Browsers will fail to launch until you pick another mode.</p>
  {/if}
  {#if readonly}
    <p class="field-hint">Display mode: {display[0].toUpperCase() + display.slice(1)} (read-only)</p>
  {/if}
  <div class="pair">
    <Input label="Screen width" type="number" min={0} bind:value={width} readonly={readonly} error={widthError} />
    <Input label="Screen height" type="number" min={0} bind:value={height} readonly={readonly} error={heightError} />
  </div>
</div>

<style>
  .warning {
    margin: 0;
    color: var(--warning-fg);
  }

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
