<script lang="ts">
  import Button from '$lib/ui/Button.svelte';
  import Switch from '$lib/ui/Switch.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { engineOf, type SectionProps } from '../form';

  let { settings, readonly = false, errors, builds, library, profileExtensions, onExtensions, extUnsupported, isTemplate }: SectionProps = $props();

  let addId = $state('');
  const engine = $derived(engineOf(settings, builds));
  const byId = $derived(new Map(library.map((ext) => [ext.id, ext])));

  const addOptions = $derived(
    library
      .filter((ext) => !profileExtensions.some((row) => row.extension_id === ext.id))
      .map((ext) => ({
        value: ext.id,
        label: ext.engine === engine ? ext.name : `${ext.name} (${ext.engine === 'chromium' ? 'Chromium' : 'Camoufox'} only)`,
        disabled: ext.engine !== engine,
      })),
  );

  function versionOptions(extId: string) {
    const ext = byId.get(extId);
    return [
      { value: 'auto', label: 'Auto-update' },
      ...((ext?.versions ?? []).map((v) => ({ value: v.version, label: v.version }))),
    ];
  }

  function setVersion(extId: string, value: string) {
    onExtensions(
      profileExtensions.map((row) =>
        row.extension_id === extId
          ? value === 'auto'
            ? { extension_id: extId, auto_update: true, pinned_version: null }
            : { extension_id: extId, auto_update: false, pinned_version: value }
          : row,
      ),
    );
  }

  function enable() {
    if (!addId) return;
    onExtensions([...profileExtensions, { extension_id: addId, auto_update: true, pinned_version: null }]);
    addId = '';
  }
</script>

{#if isTemplate}
  <p class="field-hint">Extensions are enabled per Profile, in the Profile editor.</p>
{:else}
  <div class="fields">
    <Switch label="brr companion extension" bind:checked={settings.companion} disabled={readonly} hint="Lets brr control tabs in this profile's browser through the extension." />
    <Table
      caption="Enabled extensions"
      columns={[
        { key: 'name', label: 'Name' },
        { key: 'version', label: 'Version', width: '200px' },
        { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
      ]}
      rows={profileExtensions}
      key={(row) => row.extension_id}
      loading={false}
      error={null}
      onretry={() => {}}
      empty={{ title: 'No extensions enabled' }}
      flush
    >
      {#snippet row(item)}
        <td>
          {byId.get(item.extension_id)?.name ?? item.extension_id}
          {#if byId.get(item.extension_id) && byId.get(item.extension_id)?.engine !== engine}
            <span class="field-hint">({byId.get(item.extension_id)?.engine === 'chromium' ? 'Chromium' : 'Camoufox'} only)</span>
          {/if}
        </td>
        <td>
          <Select
            label="Version for {byId.get(item.extension_id)?.name ?? item.extension_id}"
            hideLabel
            value={item.auto_update ? 'auto' : (item.pinned_version ?? 'auto')}
            onchange={(value) => setVersion(item.extension_id, value)}
            options={versionOptions(item.extension_id)}
            disabled={readonly}
          />
        </td>
        <td>
          <Button
            variant="ghost"
            size="sm"
            iconOnly
            icon="x"
            aria-label="Disable {byId.get(item.extension_id)?.name ?? item.extension_id}"
            disabled={readonly}
            onclick={() => onExtensions(profileExtensions.filter((row) => row.extension_id !== item.extension_id))}
          />
        </td>
      {/snippet}
    </Table>

    <div class="add">
      <Select label="Add extension" bind:value={addId} options={[{ value: '', label: 'Choose an extension' }, ...addOptions]} disabled={readonly} />
      <Button disabled={readonly || !addId} onclick={enable}>Enable</Button>
    </div>

    {#if extUnsupported}
      <p class="danger" role="alert">This profile's Build can't load extensions.</p>
    {:else if errors['extensions'] || errors['settings.extensions']}
      <p class="danger" role="alert">{errors['extensions'] ?? errors['settings.extensions']}</p>
    {/if}
  </div>
{/if}

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .add {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--space-2);
    align-items: end;
  }

  .danger {
    padding: var(--space-3);
    background: var(--danger-bg);
    border: 1px solid var(--danger-border);
    border-radius: var(--radius-md);
    color: var(--danger-fg);
  }
</style>
