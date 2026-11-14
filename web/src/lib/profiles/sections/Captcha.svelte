<script lang="ts">
  import { untrack } from 'svelte';
  import Select from '$lib/ui/Select.svelte';
  import type { SectionProps } from '../form';

  let { settings, readonly = false, defaults }: SectionProps = $props();

  let policy = $state<string>(untrack(() => (settings.captcha ?? '')));

  $effect(() => {
    settings.captcha = policy === '' ? null : (policy as typeof settings.captcha);
  });
</script>

<Select
  label="Captcha policy"
  bind:value={policy}
  disabled={readonly}
  hint="Solving needs a provider key on the Captcha page."
  options={[
    { value: '', label: `Default${defaults ? ` (${defaults.captcha.default_policy.replace('_', ' ')})` : ''}` },
    { value: 'off', label: 'Off' },
    { value: 'on_demand', label: 'On demand (Detect only; solve when an agent asks)' },
    { value: 'auto', label: 'Auto (Detect and solve)' },
    { value: 'human', label: 'Human (Detect, then wait for a person)' },
  ]}
/>
