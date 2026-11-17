<script lang="ts">
  import { untrack } from 'svelte';
  import Input from '$lib/ui/Input.svelte';
  import type { SectionProps } from '../form';

  let { settings, readonly = false }: SectionProps = $props();

  let locale = $state(untrack(() => (settings.locale ?? '')));
  let timezone = $state(untrack(() => (settings.timezone ?? '')));

  const timezones = Intl.supportedValuesOf('timeZone');

  $effect(() => {
    settings.locale = locale.trim() === '' ? null : locale;
  });
  $effect(() => {
    settings.timezone = timezone.trim() === '' ? null : timezone;
  });
</script>

<div class="pair">
  <Input label="Locale" bind:value={locale} readonly={readonly} placeholder={readonly ? 'Not set' : 'Example: en-US'} />
  <div>
    <Input label="Timezone" bind:value={timezone} readonly={readonly} placeholder={readonly ? 'Not set' : 'Example: UTC'} list="timezones" />
    <datalist id="timezones">
      {#each timezones as zone (zone)}<option value={zone}></option>{/each}
    </datalist>
  </div>
</div>

<style>
  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }
</style>
