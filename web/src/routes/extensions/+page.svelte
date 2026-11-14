<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import AddExtensionDialog from '$lib/extensions/AddExtensionDialog.svelte';
  import { clock } from '$lib/clock.svelte';
  import { relative } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import type { components } from '$lib/api/schema';

  type ExtensionView = components['schemas']['ExtensionView'];

  const SOURCE_LABELS: Record<ExtensionView['source'], string> = {
    cws: 'Chrome Web Store',
    amo: 'Firefox Add-ons',
    local: 'Local file',
  };

  let adding = $state(false);
  let updating = $state<string | null>(null);

  const list = liveQuery(async () => (await unwrap(api.GET('/v1/extensions'))).extensions, { kinds: ['extension.*'] });

  function updatedAt(ext: ExtensionView): number | null {
    if (ext.versions.length === 0) return null;
    return Math.max(...ext.versions.map((v) => v.fetched_at));
  }

  async function checkUpdate(ext: ExtensionView) {
    if (updating) return;
    updating = ext.id;
    try {
      const out = await unwrap(api.POST('/v1/extensions/{extension_id}/update', { params: { path: { extension_id: ext.id } }, body: {} }));
      toast.success(out.updated ? `Updated to ${out.to}` : 'Already up to date');
    } catch (e) {
      toast.apiError(e, "Couldn't check for updates");
    } finally {
      updating = null;
    }
  }

  async function remove(ext: ExtensionView) {
    const using = ext.profiles.map((p) => p.name).join(', ');
    const ok = await confirm({
      title: `Remove ${ext.name}?`,
      description: using ? `It's turned on for ${using}. Removing it turns it off there.` : "It's removed from your extensions.",
      confirmLabel: 'Remove extension',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await unwrap(api.DELETE('/v1/extensions/{extension_id}', { params: { path: { extension_id: ext.id } } }));
      toast.success('Extension removed');
      list.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't remove the extension");
    }
  }
</script>

<PageHeader title="Extensions" description="Add-ons your profiles can use. Add one here, then turn it on in a profile's settings.">
  {#snippet actions()}
    <Button variant="primary" icon="plus" onclick={() => (adding = true)}>Add extension</Button>
  {/snippet}
</PageHeader>

<AddExtensionDialog bind:open={adding} onadded={() => list.reload()} />

<Table
  caption="Extensions"
  columns={[
    { key: 'name', label: 'Extension', width: '28%' },
    { key: 'engine', label: 'Works in', width: '14%' },
    { key: 'version', label: 'Version', width: '14%' },
    { key: 'profiles', label: 'Used by', width: '14%' },
    { key: 'updated', label: 'Updated', width: '14%' },
    { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
  ]}
  rows={list.data}
  key={(row: ExtensionView) => row.id}
  loading={list.loading}
  error={list.error}
  onretry={() => list.reload()}
  empty={{ icon: 'puzzle', title: 'No extensions yet', description: 'Add one from the Chrome Web Store, Firefox Add-ons or a file. Then turn it on for a profile.' }}
>
  {#snippet row(ext: ExtensionView)}
    <td>
      <span class="name">
        <span class="truncate">{ext.name}</span>
        <Badge>{SOURCE_LABELS[ext.source]}</Badge>
      </span>
    </td>
    <td>{ext.engine === 'camoufox' ? 'Camoufox' : 'Chrome-based browsers'}</td>
    <td class="num">{ext.latest_version ?? '—'}</td>
    <td class="truncate" title={ext.profiles.map((p) => p.name).join(', ') || undefined}>{ext.profiles.length === 0 ? 'No profile yet' : ext.profiles.map((p) => p.name).join(', ')}</td>
    <td>{updatedAt(ext) ? relative(updatedAt(ext) as number, clock.now) : '—'}</td>
    <td>
      <Menu
        label="Actions for {ext.name}"
        items={[
          { label: 'Check for updates', onselect: () => checkUpdate(ext) },
          { label: 'Remove…', onselect: () => remove(ext) },
        ]}
      />
    </td>
  {/snippet}
</Table>

<style>
  .name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
</style>
