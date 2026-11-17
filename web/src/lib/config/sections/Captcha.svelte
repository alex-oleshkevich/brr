<script lang="ts">
  import { untrack } from 'svelte';
  import Select from '$lib/ui/Select.svelte';
  import { settingsHref } from '$lib/settings/sections';
  import type { SectionProps } from '../form';

  let { settings, readonly = false, defaults }: SectionProps = $props();

  let policy = $state<string>(untrack(() => (settings.captcha ?? '')));

  $effect(() => {
    settings.captcha = policy === '' ? null : (policy as typeof settings.captcha);
  });
</script>

<div class="fields">
  <Select
    label="Captcha policy"
    bind:value={policy}
    disabled={readonly}
    options={[
      { value: '', label: `Default${defaults ? ` (${defaults.captcha.default_policy.replace('_', ' ')})` : ''}` },
      { value: 'off', label: 'Off' },
      { value: 'on_demand', label: 'On demand (Detect only; solve when an agent asks)' },
      { value: 'auto', label: 'Auto (Detect and solve)' },
      { value: 'human', label: 'Human (Detect, then wait for a person)' },
    ]}
  />
  {#if !readonly}
    <p class="field-hint">Solving needs a provider key. Set it in <a href={settingsHref('captcha')}>Settings → Captcha</a>.</p>
  {/if}
</div>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
</style>
