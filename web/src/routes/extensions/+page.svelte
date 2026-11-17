<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import AddExtensionDialog from '$lib/extensions/AddExtensionDialog.svelte';
  import AddToTemplatesDialog from '$lib/extensions/AddToTemplatesDialog.svelte';
  import { templateTitle } from '$lib/config/templates';

  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
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
  let addingToTemplates = $state<ExtensionView | null>(null);
  let addToTemplatesOpen = $state(false);

  const list = liveQuery(async () => (await unwrap(api.GET('/v1/extensions'))).extensions, { kinds: ['extension.*'] });
  const templates = liveQuery(async () => (await unwrap(api.GET('/v1/templates'))).templates, { kinds: [] });
  const templateName = (id: string) => {
    const found = (templates.data ?? []).find((t) => t.id === id);
    return found ? templateTitle(found) : id;
  };



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

  function openAddToTemplates(ext: ExtensionView) {
    addingToTemplates = ext;
    addToTemplatesOpen = true;
  }

  let refusing = $state<ExtensionView | null>(null);
  let refusalOpen = $state(false);

  async function remove(ext: ExtensionView) {
    // The daemon refuses while a Profile enables it; say so up front instead of confirming first.
    if (ext.profiles.length > 0) {
      refusing = ext;
      refusalOpen = true;
      return;
    }
    const templateNames = (ext.template_ids ?? []).map(templateName);
    const ok = await confirm({
      title: `Remove ${ext.name}?`,
      description: templateNames.length > 0 ? `It is removed from these templates, and they lose it: ${templateNames.join(', ')}.` : "It's removed from your extensions.",
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

<PageHeader title="Extensions" description="Add-ons your templates and profiles can use. Add one here, then turn it on in a template or profile.">
  {#snippet actions()}
    <Button variant="primary" icon="plus" onclick={() => (adding = true)}>Add extension</Button>
  {/snippet}
</PageHeader>

<AddExtensionDialog bind:open={adding} onadded={() => list.reload()} />
<AddToTemplatesDialog bind:open={addToTemplatesOpen} extension={addingToTemplates} onsaved={() => { templates.reload(); list.reload(); }} />

{#if refusalOpen && refusing}
  <Dialog bind:open={refusalOpen} title="Can't remove {refusing.name} yet">
    <p>These profiles have it turned on. Turn it off in each profile first, then remove it:</p>
    <ul class="profiles">
      {#each refusing.profiles as p (p.id)}
        <li><a href="/profiles/{p.id}?tab=settings">{p.name}</a></li>
      {/each}
    </ul>
    {#snippet footer()}
      <Button variant="primary" onclick={() => (refusalOpen = false)}>Close</Button>
    {/snippet}
  </Dialog>
{/if}

<Table
  caption="Extensions"
  columns={[
    { key: 'name', label: 'Name', width: '24%' },
    { key: 'engine', label: 'Engine', width: '14%' },
    { key: 'source', label: 'Source', width: '14%' },
    { key: 'version', label: 'Version', width: '10%' },
    { key: 'templates', label: 'Templates using it', width: '17%' },
    { key: 'profiles', label: 'Profiles using it', width: '17%' },
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
      <span class="truncate">{ext.name}</span>
    </td>
    <td>{ext.engine === 'camoufox' ? 'Camoufox' : 'Chromium'}</td>
    <td>{SOURCE_LABELS[ext.source]}</td>
    <td class="num">{ext.latest_version ?? '—'}</td>
    <td class="truncate" title={(ext.template_ids ?? []).map(templateName).join(', ') || undefined}>
      {(ext.template_ids ?? []).length === 0 ? 'None' : (ext.template_ids ?? []).map(templateName).join(', ')}
    </td>
    <td class="truncate" title={ext.profiles.map((p) => p.name).join(', ') || undefined}>{ext.profiles.length === 0 ? 'None' : ext.profiles.map((p) => p.name).join(', ')}</td>

    <td>
      <Menu
        label="Actions for {ext.name}"
        items={[
          {
            label: updating === ext.id ? 'Updating…' : 'Update',
            disabled: updating === ext.id || ext.source === 'local',
            hint: ext.source === 'local' ? "Local-file extensions can't be checked for updates." : undefined,
            onselect: () => checkUpdate(ext),
          },
          { label: 'Add to templates…', onselect: () => openAddToTemplates(ext) },
          { label: 'Remove…', onselect: () => remove(ext) },
        ]}
      />
    </td>
  {/snippet}
</Table>

<style>
  .profiles {
    margin: var(--space-3) 0 0;
    padding-left: var(--space-5);
  }
</style>


