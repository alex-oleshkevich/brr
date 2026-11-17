<script lang="ts">
  import Switch from '$lib/ui/Switch.svelte';
  import type { SectionProps } from '../form';

  let { settings, readonly = false }: SectionProps = $props();
</script>

<div class="fields">
  <Switch label="Block ads" bind:checked={settings.block_ads} disabled={readonly} hint="Adds request interception; leave off for stealth profiles" />
  <Switch label="Dismiss cookie banners" bind:checked={settings.dismiss_cookies} disabled={readonly} />
</div>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
</style>
