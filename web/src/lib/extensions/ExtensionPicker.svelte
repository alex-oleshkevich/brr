<script lang="ts">
  import Checkbox from '$lib/ui/Checkbox.svelte';
  import Select from '$lib/ui/Select.svelte';
  import type { components } from '$lib/api/schema';

  type ExtensionView = components['schemas']['ExtensionView'];
  type ProfileExtension = components['schemas']['ProfileExtension'];

  let {
    library,
    engine,
    value,
    onchange,
    readonly = false,
  }: {
    library: ExtensionView[];
    engine: 'chromium' | 'camoufox';
    value: ProfileExtension[];
    onchange: (next: ProfileExtension[]) => void;
    readonly?: boolean;
  } = $props();

  const matching = $derived(library.filter((ext) => ext.engine === engine));
  const byId = $derived(new Map(library.map((ext) => [ext.id, ext])));
  // Selected rows that can't load in this engine (or are no longer in the library) stay visible so they can be removed.
  const mismatched = $derived(value.filter((row) => byId.get(row.extension_id)?.engine !== engine));
  const engineLabel = $derived(engine === 'camoufox' ? 'Camoufox' : 'Chromium');

  const selected = (ext: ExtensionView) => value.find((row) => row.extension_id === ext.id);

  function toggle(ext: ExtensionView, checked: boolean) {
    if (checked) onchange([...value, { extension_id: ext.id, auto_update: true, pinned_version: null }]);
    else onchange(value.filter((row) => row.extension_id !== ext.id));
  }

  function setVersion(ext: ExtensionView, next: string) {
    onchange(
      value.map((row) =>
        row.extension_id === ext.id
          ? next === 'auto'
            ? { extension_id: ext.id, auto_update: true, pinned_version: null }
            : { extension_id: ext.id, auto_update: false, pinned_version: next }
          : row,
      ),
    );
  }
</script>

{#if matching.length === 0 && mismatched.length === 0}
  <p class="field-hint">{readonly ? 'Not set' : `No ${engineLabel} extensions in the library yet. Add one on the Extensions page first.`}</p>
{:else}
  <div class="picker">
    {#each mismatched as row (row.extension_id)}
      <div class="item">
        <Checkbox
          label="{byId.get(row.extension_id)?.name ?? row.extension_id} — Wrong engine — remove it"
          checked
          disabled={readonly}
          onchange={(checked) => {
            if (!checked) onchange(value.filter((r) => r.extension_id !== row.extension_id));
          }}
        />
        <span></span>
      </div>
    {/each}
    {#each matching as ext (ext.id)}
      {@const row = selected(ext)}
      <div class="item">
        <Checkbox label={ext.name} checked={Boolean(row)} disabled={readonly} onchange={(checked) => toggle(ext, checked)} />
        {#if row}
          <Select
            label="Version for {ext.name}"
            hideLabel
            value={row.auto_update ? 'auto' : (row.pinned_version ?? 'auto')}
            onchange={(next) => setVersion(ext, next)}
            options={[{ value: 'auto', label: 'Auto-update' }, ...ext.versions.map((v) => ({ value: v.version, label: v.version }))]}
            disabled={readonly}
          />
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .item {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 200px;
    gap: var(--space-3);
    align-items: center;
  }
</style>
