<script lang="ts">
  import { goto } from '$app/navigation';
  import { ApiError } from '$lib/api/client';
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import NewProfileDialog from '$lib/profiles/NewProfileDialog.svelte';
  import { engineOf } from '$lib/profiles/form';
  import { clock } from '$lib/clock.svelte';
  import { relative } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import { templatePurpose, templateTitle, type Template } from '$lib/profiles/templates';

  let creating = $state(false);
  let createTemplateId = $state<string | undefined>(undefined);
  let duplicating = $state<Template | null>(null);
  let duplicateName = $state('');
  let duplicateOpen = $state(false);
  let duplicateBusy = $state(false);
  let duplicateError = $state<string | null>(null);

  const list = liveQuery(async () => (await unwrap(api.GET('/v1/templates'))).templates, { kinds: [] });
  // The daemon emits no template events, so changes made elsewhere (another tab, the CLI) show up when the window regains focus.
  $effect(() => {
    const refresh = () => list.reload();
    window.addEventListener('focus', refresh);

    return () => window.removeEventListener('focus', refresh);
  });

  const buildsQuery = liveQuery(async () => (await unwrap(api.GET('/v1/builds'))).builds, { kinds: [] });

  function newFromTemplate(t: Template) {
    createTemplateId = t.id;
    creating = true;
  }

  function openDuplicate(t: Template) {
    duplicating = t;
    duplicateName = `${t.name}-copy`;
    duplicateError = null;
    duplicateOpen = true;
  }

  async function duplicate(event: SubmitEvent) {
    event.preventDefault();
    if (!duplicating || !duplicateName.trim() || duplicateBusy) return;
    duplicateBusy = true;
    duplicateError = null;
    try {
      const created = await unwrap(api.POST('/v1/templates', { body: { name: duplicateName.trim(), settings: duplicating.settings } }));
      toast.success('Template duplicated');
      duplicateOpen = false;
      list.reload();
      await goto(`/templates/${created.id}`);
    } catch (e) {
      if (e instanceof ApiError && (e.code === 'Conflict' || (e.code === 'InvalidArgument' && e.details?.field === 'name'))) {
        duplicateError = e.message;
      } else {
        toast.apiError(e, "Couldn't duplicate the template");
      }
    } finally {
      duplicateBusy = false;
    }
  }

  async function remove(t: Template) {
    const ok = await confirm({
      title: `Delete ${t.name}?`,
      description: 'Profiles already created from it keep their settings.',
      confirmLabel: 'Delete template',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await unwrap(api.DELETE('/v1/templates/{template}', { params: { path: { template: t.id } } }));
      toast.success('Template deleted');
      list.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't delete the template");
    }
  }
</script>

<NewProfileDialog bind:open={creating} templateId={createTemplateId} />

<Dialog bind:open={duplicateOpen} title="Duplicate template">
  <form id="duplicate-form" onsubmit={duplicate}>
    <Input label="Name" bind:value={duplicateName} required error={duplicateError} />
  </form>
  {#snippet footer()}
    <Button onclick={() => (duplicateOpen = false)} disabled={duplicateBusy}>Cancel</Button>
    <Button variant="primary" type="submit" form="duplicate-form" loading={duplicateBusy} disabled={!duplicateName.trim()}>Duplicate</Button>
  {/snippet}
</Dialog>

<p class="intro">Starting points for new profiles. A profile copies its template once and doesn't follow later changes.</p>

<Table
  caption="Templates"
  columns={[
    { key: 'name', label: 'Template', width: '52%' },
    { key: 'engine', label: 'Engine', width: '14%' },
    { key: 'display', label: 'Display', width: '14%' },
    { key: 'updated', label: 'Updated', width: '14%' },
    { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
  ]}
  rows={list.data}
  key={(row: Template) => row.id}
  loading={list.loading}
  error={list.error}
  onretry={() => list.reload()}
  empty={{ icon: 'template', title: 'No templates yet', description: 'Templates appear here once they are created.' }}
>
  {#snippet row(t: Template)}
    <td>
      <span class="name">
        <a href="/templates/{t.id}">{templateTitle(t)}</a>
        {#if t.builtin}<Badge>Built-in</Badge>{/if}
      </span>
      {#if templatePurpose(t)}<span class="purpose">{templatePurpose(t)}</span>{/if}
    </td>
    <td>{engineOf(t.settings, buildsQuery.data ?? []) === 'camoufox' ? 'Camoufox' : 'Chromium'}</td>
    <td>{t.settings.display[0].toUpperCase() + t.settings.display.slice(1)}</td>
    <td>{t.builtin ? '—' : relative(t.updated_at, clock.now)}</td>
    <td>
      <Menu
        label="Actions for {t.name}"
        items={[
          { label: 'New profile from template', onselect: () => newFromTemplate(t) },
          { label: 'Duplicate…', onselect: () => openDuplicate(t) },
          ...(!t.builtin
            ? [
                { label: 'Edit', onselect: () => goto(`/templates/${t.id}`) },
                { label: 'Delete…', onselect: () => remove(t) },
              ]
            : []),
        ]}
      />
    </td>
  {/snippet}
</Table>

<style>
  .intro {
    margin-bottom: var(--space-4);
    color: var(--text-secondary);
  }

  .name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .name a {
    font-weight: 500;
  }

  .purpose {
    display: block;
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }
</style>
