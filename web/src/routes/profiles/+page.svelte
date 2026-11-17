<script lang="ts">
  import { goto } from '$app/navigation';
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import LaunchDialog from '$lib/instances/LaunchDialog.svelte';
  import ImportProfileDialog from '$lib/profiles/ImportProfileDialog.svelte';
  import NewProfileDialog from '$lib/profiles/NewProfileDialog.svelte';
  import DeleteProfileDialog from '$lib/profiles/DeleteProfileDialog.svelte';
  import ExportProfileDialog from '$lib/profiles/ExportProfileDialog.svelte';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { bytes, relative } from '$lib/format';
  import { clock } from '$lib/clock.svelte';
  import StatusDot from '$lib/ui/StatusDot.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import type { Snippet } from 'svelte';
  import type { components } from '$lib/api/schema';
  import { profileNameError } from '$lib/profiles/restore';
  import { templateNameLabel, templateShortNameLabel, uniqueCopyName } from '$lib/config/templates';
  import { settingsHref } from '$lib/settings/sections';

  type Profile = components['schemas']['Profile'];

  let filter = $state('');
  let creating = $state(false);
  let importing = $state(false);
  let createFromTemplate = $state<string | undefined>(undefined);
  let launchingFor = $state<string | null>(null);
  let launching = $state(false);
  let showSystem = $state(false);
  let deleting = $state<Profile | null>(null);
  let deleteOpen = $state(false);
  let exporting = $state<Profile | null>(null);
  let exportOpen = $state(false);
  let duplicating = $state<Profile | null>(null);
  let duplicateName = $state('');
  let duplicateOpen = $state(false);
  let duplicateBusy = $state(false);
  let backingUp = $state<string | null>(null);

  const list = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, {
    kinds: ['profile.*', 'instance.state_changed', 'login.check_result'],
  });
  const settingsQuery = liveQuery(async () => unwrap(api.GET('/v1/settings')), { kinds: [] });
  const builtinName = $derived(settingsQuery.data?.settings.oneshot.profile ?? 'oneshot');
  const hasPassphrase = $derived(settingsQuery.data?.secrets['backup.passphrase']?.set === true);
  const isBuiltin = (profile: Profile) => profile.builtin || profile.name === builtinName;
  const usage = liveQuery(async () => (await unwrap(api.GET('/v1/storage/usage'))).profiles, {
    kinds: ['profile.*', 'artifact.*', 'download.*', 'retention.sweep_completed'],
  });
  const usedBytes = $derived(new Map((usage.data ?? []).map((u) => [u.profile_id, u.bytes])));

  const instances = liveQuery(
    async () => (await unwrap(api.GET('/v1/instances', { params: { query: { limit: 500 } } }))).instances,
    { kinds: ['instance.*'] },
  );
  const lastRunBy = $derived.by(() => {
    const found = new Map<string, number>();
    for (const i of instances.data ?? []) {
      if (!i.profile_id) continue;
      const at = i.started_at ?? i.queued_at;
      if (at > (found.get(i.profile_id) ?? 0)) found.set(i.profile_id, at);
    }
    return found;
  });
  type LiveInstance = { id: string; queued: boolean };
  const liveBy = $derived.by(() => {
    const found = new Map<string, LiveInstance>();
    for (const i of instances.data ?? []) {
      if (!i.profile_id) continue;
      if (!['queued', 'starting', 'running', 'idle', 'reaping'].includes(i.state)) continue;
      const queued = i.state === 'queued';
      if (!found.has(i.profile_id) || !queued) found.set(i.profile_id, { id: i.id, queued });
    }
    return found;
  });

  const matching = $derived(
    (list.data ?? []).filter((p) => {
      const q = filter.trim().toLowerCase();
      if (!q) return true;
      return p.name.toLowerCase().includes(q) || p.tags.some((tag) => tag.toLowerCase().includes(q));
    }),
  );
  const rows = $derived(matching.filter((p) => !isBuiltin(p)));
  const systemRows = $derived(matching.filter(isBuiltin));
  const isEmpty = $derived(list.data !== undefined && list.data.every(isBuiltin));
  const filtering = $derived(filter.trim() !== '');

  function openCreate(templateId?: string) {
    createFromTemplate = templateId;
    creating = true;
  }

  function launch(profile: Profile) {
    launchingFor = profile.id;
    launching = true;
  }

  function remove(profile: Profile) {
    deleting = profile;
    deleteOpen = true;
  }

  function openExport(profile: Profile) {
    exporting = profile;
    exportOpen = true;
  }

  const duplicateNameError = $derived(duplicateName.trim() === '' ? null : profileNameError(duplicateName.trim()));

  function openDuplicate(profile: Profile) {
    duplicating = profile;
    const taken = new Set((list.data ?? []).map((p) => p.name));
    duplicateName = uniqueCopyName(profile.name.slice(0, 58), taken);
    duplicateOpen = true;
  }

  async function duplicate(event: SubmitEvent) {
    event.preventDefault();
    const source = duplicating;
    const templateId = source?.template_id;
    if (!source || !templateId || duplicateNameError || !duplicateName.trim() || duplicateBusy) return;
    duplicateBusy = true;
    try {
      const created = await unwrap(
        api.POST('/v1/profiles', { body: { name: duplicateName.trim(), template: templateId, kind: source.kind } }),
      );
      await unwrap(
        api.PATCH('/v1/profiles/{profile}', {
          params: { path: { profile: created.id } },
          body: { profile: created.id, settings: source.settings, tags: source.tags, notes: source.notes },
        }),
      );
      const ext = await unwrap(api.GET('/v1/profiles/{profile}/extensions', { params: { path: { profile: source.id } } }));
      await unwrap(
        api.PUT('/v1/profiles/{profile}/extensions', { params: { path: { profile: created.id } }, body: { extensions: ext.extensions } }),
      );
      toast.success('Profile duplicated');
      duplicateOpen = false;
      list.reload();
      await goto(`/profiles/${created.id}`);
    } catch (e) {
      toast.apiError(e, "Couldn't duplicate the profile");
    } finally {
      duplicateBusy = false;
    }
  }

  async function backUpNow(profile: Profile) {
    if (settingsQuery.data && !hasPassphrase) {
      toast.show({
        tone: 'warning',
        title: 'Set a backup passphrase in Settings → Storage first',
        action: { label: 'Open Settings → Storage', onclick: () => void goto(settingsHref('storage')) },
      });
      return;
    }
    if (backingUp) return;
    backingUp = profile.id;
    try {
      await unwrap(api.POST('/v1/profiles/{profile}/backups', { params: { path: { profile: profile.id } }, body: {} }));
      toast.success(`Backed up ${profile.name}`);
    } catch (e) {
      toast.apiError(e, "Couldn't back up the profile");
    } finally {
      backingUp = null;
    }
  }
</script>

<PageHeader title="Profiles" description="Each profile is a saved browser identity: its settings, logins and history.">
  {#snippet actions()}
    <Button onclick={() => (importing = true)}>Import</Button>
    {#if !isEmpty}
      <Button variant="primary" icon="plus" onclick={() => openCreate()}>New profile</Button>
    {/if}
  {/snippet}
</PageHeader>

<NewProfileDialog bind:open={creating} templateId={createFromTemplate} />
<ImportProfileDialog bind:open={importing} />
<DeleteProfileDialog bind:open={deleteOpen} profile={deleting} ondeleted={() => list.reload()} />
{#if exporting}<ExportProfileDialog bind:open={exportOpen} profileId={exporting.id} profileName={exporting.name} />{/if}
{#if launchingFor}<LaunchDialog bind:open={launching} profileId={launchingFor} />{/if}

{#if duplicateOpen}
  <Dialog bind:open={duplicateOpen} title="Duplicate profile">
    <form id="duplicate-profile-form" onsubmit={duplicate}>
      <div class="fields">
        <Input label="Name" bind:value={duplicateName} required error={duplicateNameError} />
        <p class="field-hint">Copies the settings and extensions. Browser data (cookies, history) starts fresh.</p>
      </div>
    </form>
    {#snippet footer()}
      <Button onclick={() => (duplicateOpen = false)} disabled={duplicateBusy}>Cancel</Button>
      <Button variant="primary" type="submit" form="duplicate-profile-form" loading={duplicateBusy} disabled={!duplicateName.trim() || duplicateNameError !== null}>Duplicate</Button>
    {/snippet}
  </Dialog>
{/if}

<div class="stack">
  {#if !isEmpty}
    <div class="filter">
      <Input type="search" label="Filter profiles" hideLabel placeholder="Filter by name or tag" bind:value={filter} />
    </div>
  {/if}

  {#snippet newProfileAction()}
    <Button variant="primary" onclick={() => openCreate()}>New profile</Button>
  {/snippet}

  {#snippet profileRow(profile: Profile)}
    {@const liveInstance = liveBy.get(profile.id)}
    {@const lastRun = lastRunBy.get(profile.id)}
    <td>
      <span class="name">
        {#if liveInstance && !liveInstance.queued}<StatusDot tone="success" label="Running" showLabel={false} />{/if}
        <a class="truncate" href="/profiles/{profile.id}">{profile.name}</a>
        {#if profile.pinned}<Badge>Pinned</Badge>{/if}
      </span>
    </td>
    <td>{profile.kind === 'persistent' ? 'Keeps data' : 'Fresh each time'}</td>
    <td class="truncate" title={profile.template_name ? templateNameLabel(profile.template_name) : undefined}>
      {#if profile.template_name && !profile.template_id}
        <span class="deleted">{templateShortNameLabel(profile.template_name)} (deleted)</span>
      {:else}
        {templateShortNameLabel(profile.template_name)}
      {/if}
    </td>
    <td>{lastRun ? relative(lastRun, clock.now) : 'Never'}</td>
    <td class="num right">
      <span class="disk">
        {#if profile.disk_quota_bytes != null && (usedBytes.get(profile.id) ?? 0) > profile.disk_quota_bytes}
          <span title="Launches are refused until usage drops below {bytes(profile.disk_quota_bytes)}"><Badge tone="danger">Over quota</Badge></span>
        {/if}
        {bytes(usedBytes.get(profile.id) ?? null)}
      </span>
    </td>
    <td>{#if profile.login_status}<StateBadge kind="login" state={profile.login_status} />{:else}—{/if}</td>
    <td>
      <Menu
        label="Actions for {profile.name}"
        items={[
          ...(liveInstance
            ? [{ label: liveInstance.queued ? 'Waiting to start' : 'Open browser', onselect: () => goto(`/browsers/${liveInstance.id}`) }]
            : [{ label: 'Launch', onselect: () => launch(profile) }]),
          {
            label: 'Duplicate…',
            disabled: !profile.template_id,
            hint: profile.template_id
              ? undefined
              : profile.template_name
                ? `The template ${templateNameLabel(profile.template_name)} was deleted`
                : 'It did not come from a template.',
            onselect: () => openDuplicate(profile),
          },
          {
            label: 'Export…',
            disabled: profile.kind !== 'persistent' || Boolean(liveInstance),
            hint: profile.kind !== 'persistent' ? 'Only profiles that keep data can be exported.' : liveInstance ? 'Stop the profile first.' : undefined,
            onselect: () => openExport(profile),
          },
          {
            label: backingUp === profile.id ? 'Backing up…' : 'Back up now',
            disabled: profile.kind !== 'persistent' || Boolean(liveInstance) || backingUp === profile.id,
            hint: profile.kind !== 'persistent' ? 'Fresh-each-time profiles keep no data.' : liveInstance ? 'Stop the profile first.' : undefined,
            onselect: () => backUpNow(profile),
          },
          ...(!isBuiltin(profile)
            ? [
                'separator' as const,
                {
                  label: liveInstance ? 'Delete… (stop it first)' : 'Delete…',
                  tone: 'danger' as const,
                  disabled: Boolean(liveInstance),
                  hint: 'A running profile can\'t be deleted. Stop the profile first.',
                  onselect: () => remove(profile),
                },
              ]
            : []),
        ]}
      />
    </td>
  {/snippet}

  {#snippet profilesTable(caption: string, tableRows: Profile[] | undefined, emptyState: { title: string; description?: string; icon?: 'search' | 'user'; action?: Snippet })}
    <Table
      {caption}
      columns={[
        { key: 'name', label: 'Name', width: '22%' },
        { key: 'kind', label: 'Kind', width: '13%' },
        { key: 'template', label: 'Template', width: '18%' },
        { key: 'last-run', label: 'Last run', width: '12%' },
        { key: 'disk', label: 'Disk use', width: '17%', align: 'right' },
        { key: 'login', label: 'Login status', width: '14%' },
        { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
      ]}
      rows={tableRows}
      key={(row: Profile) => row.id}
      loading={list.loading}
      error={list.error}
      onretry={() => list.reload()}
      empty={emptyState}
      row={profileRow}
    />
  {/snippet}

  {@render profilesTable(
    'Profiles',
    list.data === undefined ? undefined : rows,
    filtering
      ? { icon: 'search', title: 'No matching profiles', description: 'Try another name or tag.' }
      : { icon: 'user', title: 'No profiles yet', description: 'Create one from a template to launch a browser.', action: newProfileAction },
  )}

  {#if (list.data ?? []).some(isBuiltin)}
    <div>
      <Button variant="ghost" size="sm" aria-expanded={showSystem} aria-controls="system-profiles" onclick={() => (showSystem = !showSystem)}>
        {showSystem ? 'Hide system profiles' : 'Show system profiles'}
      </Button>
    </div>
    {#if showSystem}
      <section id="system-profiles" class="system" aria-labelledby="system-profiles-title">
        <h2 id="system-profiles-title">System</h2>
        <p>These profiles are built in. brr uses them itself, for example for one-off jobs, so there is rarely a reason to change them.</p>
        {@render profilesTable('System profiles', systemRows, { title: 'No matching system profiles' })}
      </section>
    {/if}
  {/if}
</div>

<style>
  .system h2 {
    font-size: var(--text-base-size);
    line-height: var(--text-base-line);
  }

  .system p {
    margin: var(--space-1) 0 var(--space-3);
    color: var(--text-secondary);
  }

  .deleted {
    color: var(--text-tertiary);
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .filter {
    width: 280px;
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .name {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .name a {
    flex: 0 1 auto;
    min-width: 0;
    font-weight: 500;
  }

  .name :global(.badge) {
    flex: none;
  }

  .disk {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    white-space: nowrap;
  }
</style>
