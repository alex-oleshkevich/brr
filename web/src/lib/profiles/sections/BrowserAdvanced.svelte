<script lang="ts">
  import { untrack } from 'svelte';
  import Button from '$lib/ui/Button.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Textarea from '$lib/ui/Textarea.svelte';
  import { fieldError, type SectionProps } from '../form';

  let { settings, readonly = false, errors, reportInvalid }: SectionProps = $props();

  let argsText = $state(untrack(() => (settings.args.join('\n'))));
  let envRows = $state(untrack(() => (Object.entries(settings.env).map(([key, value]) => ({ key, value })))));
  let prefsText = $state(untrack(() => (JSON.stringify(settings.prefs, null, 2))));
  let prefsError = $state<string | null>(null);
  let prefsTouched = $state(false);

  $effect(() => {
    settings.args = argsText.split('\n').map((line) => line.trim()).filter(Boolean);
  });
  $effect(() => {
    settings.env = Object.fromEntries(envRows.filter((row) => row.key.trim()).map((row) => [row.key.trim(), row.value]));
  });

  $effect(() => {
    try {
      const parsed: unknown = JSON.parse(prefsText || '{}');
      if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) throw new Error('not an object');
      settings.prefs = parsed as Record<string, unknown>;
      reportInvalid('prefs', false);
      if (prefsTouched) prefsError = null;
    } catch {
      reportInvalid('prefs', true);
      if (prefsTouched) prefsError = 'Preferences must be a JSON object';
    }
  });

  function checkPrefs() {
    prefsTouched = true;
  }
</script>

<div class="fields">
  <Textarea label="Extra launch arguments" bind:value={argsText} rows={3} readonly={readonly} hint="One argument per line." error={fieldError(errors, 'settings.args', 'args')} />

  <div class="field">
    <span class="field-label">Environment variables</span>
    {#each envRows as row, index (index)}
      <div class="env-row">
        <Input label="Variable name" hideLabel bind:value={row.key} placeholder="NAME" readonly={readonly} />
        <Input label="Variable value" hideLabel bind:value={row.value} placeholder="value" readonly={readonly} />
        <Button variant="ghost" size="sm" iconOnly icon="x" aria-label="Remove variable" disabled={readonly} onclick={() => (envRows = envRows.filter((_, i) => i !== index))} />
      </div>
    {/each}
    <div><Button size="sm" icon="plus" disabled={readonly} onclick={() => (envRows = [...envRows, { key: '', value: '' }])}>Add variable</Button></div>
    {#if fieldError(errors, 'settings.env', 'env')}<p class="field-error">{fieldError(errors, 'settings.env', 'env')}</p>{/if}
  </div>

  <div onfocusout={checkPrefs} role="presentation">
    <Textarea
      label="Browser preferences (JSON)"
      bind:value={prefsText}
      rows={4}
      readonly={readonly}
      error={prefsError ?? fieldError(errors, 'settings.prefs', 'prefs')}
    />
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

  .env-row {
    display: grid;
    grid-template-columns: 1fr 1fr auto;
    gap: var(--space-2);
    align-items: end;
    width: 100%;
  }
</style>
