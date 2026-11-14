<script lang="ts">
  import { untrack } from 'svelte';
  import Input from '$lib/ui/Input.svelte';
  import { fieldError, type SectionProps } from '../form';

  let { settings, readonly = false, errors, defaults, isTemplate, quotaGb, onQuota }: SectionProps = $props();

  let downloadMax = $state<number | null>(untrack(() => (settings.download_max_mb)));
  let keepDays = $state<number | null>(untrack(() => (settings.retention.artifacts_max_age_days ?? null)));
  let artifactsGb = $state<number | null>(untrack(() => (settings.retention.artifacts_max_total_gb ?? null)));
  let quota = $state<number | null>(untrack(() => (quotaGb)));

  $effect(() => {
    settings.download_max_mb = downloadMax;
  });
  $effect(() => {
    const retention = untrack(() => settings.retention);
    settings.retention = { ...retention, artifacts_max_age_days: keepDays, artifacts_max_total_gb: artifactsGb };
  });
  $effect(() => {
    onQuota(quota);
  });
</script>

<div class="fields">
  <div class="pair">
    <Input label="Download size limit (MB)" type="number" min={0} bind:value={downloadMax} readonly={readonly} placeholder={defaults ? `Default (${defaults.retention.download_max_mb})` : undefined} error={fieldError(errors, 'settings.download_max_mb', 'download_max_mb')} />
    <Input label="Keep artifacts for (days)" type="number" min={0} bind:value={keepDays} readonly={readonly} placeholder={defaults ? `Default (${defaults.retention.artifacts_max_age_days})` : undefined} />
  </div>
  <div class="pair">
    <Input label="Artifact storage limit (GB)" type="number" min={0} bind:value={artifactsGb} readonly={readonly} placeholder={defaults ? `Default (${defaults.retention.artifacts_max_total_gb})` : undefined} />
    {#if !isTemplate}
      <Input label="Disk quota (GB)" type="number" min={0} bind:value={quota} readonly={readonly} />
    {/if}
  </div>
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
