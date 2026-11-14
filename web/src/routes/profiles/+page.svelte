<script lang="ts">
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import LaunchDialog from '$lib/instances/LaunchDialog.svelte';
  import ImportProfileDialog from '$lib/profiles/ImportProfileDialog.svelte';
  import NewProfileDialog from '$lib/profiles/NewProfileDialog.svelte';
  import DeleteProfileDialog from '$lib/profiles/DeleteProfileDialog.svelte';
  import TemplatesTab from '$lib/profiles/TemplatesTab.svelte';
  import { missingLabel, resolveBuild, resolvedLabel } from '$lib/profiles/resolve';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';
  import Table from '$lib/ui/Table.svelte';
  import Tabs from '$lib/ui/Tabs.svelte';
  import { bytes } from '$lib/format';
  import StatusDot from '$lib/ui/StatusDot.svelte';
  import type { Snippet } from 'svelte';
  import type { components } from '$lib/api/schema';

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
  // The tab lives in the address, so a reload or a shared link lands on the same tab.
  const tab = $derived(page.url.searchParams.get('tab') === 'templates' ? 'templates' : 'profiles');
  const setTab = (next: string) => goto(next === 'templates' ? '/profiles?tab=templates' : '/profiles', { replaceState: true, keepFocus: true, noScroll: true });

  const list = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, {
    kinds: ['profile.*', 'instance.state_changed', 'login.check_result'],
  });
  const buildsQuery = liveQuery(async () => (await unwrap(api.GET('/v1/builds'))).builds, { kinds: ['build.*'] });
  const settingsQuery = liveQuery(async () => (await unwrap(api.GET('/v1/settings'))).settings, { kinds: [] });
  const builtinName = $derived(settingsQuery.data?.oneshot.profile ?? 'oneshot');
  const isBuiltin = (profile: Profile) => profile.builtin || profile.name === builtinName;
  const usage = liveQuery(async () => (await unwrap(api.GET('/v1/storage/usage'))).profiles, {
    kinds: ['profile.*', 'artifact.*', 'download.*', 'retention.sweep_completed'],
  });
  const usedBytes = $derived(new Map((usage.data ?? []).map((u) => [u.profile_id, u.bytes])));

  // A persistent profile runs at most one browser, so it can't be launched again while one is live.
  const live = liveQuery(
    async () =>
      (await unwrap(api.GET('/v1/instances', { params: { query: { state: 'queued,starting,running,idle,reaping', limit: 500 } } }))).instances,
    { kinds: ['instance.*'] },
  );
  type LiveInstance = { id: string; queued: boolean };
  // One entry per profile; a started browser wins over a queued one.
  const liveBy = $derived.by(() => {
    const found = new Map<string, LiveInstance>();
    for (const i of live.data ?? []) {
      if (!i.profile_id) continue;
      const queued = i.state === 'queued';
      if (!found.has(i.profile_id) || !queued) found.set(i.profile_id, { id: i.id, queued });
    }
    return found;
  });

  const builds = $derived(buildsQuery.data ?? []);
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
</script>

<PageHeader
  title="Profiles"
  description={tab === 'templates'
    ? 'Starting points for new profiles. A profile copies its template once and does not follow later changes.'
    : 'Each profile is a saved browser identity: its settings, logins and history.'}
>
  {#snippet actions()}
    {#if tab === 'profiles'}
      <Button onclick={() => (importing = true)}>Import profile</Button>
      {#if !isEmpty}
        <Button variant="primary" icon="plus" onclick={() => openCreate()}>New profile</Button>
      {/if}
    {/if}
  {/snippet}
</PageHeader>

<NewProfileDialog bind:open={creating} templateId={createFromTemplate} />
<ImportProfileDialog bind:open={importing} />
<DeleteProfileDialog bind:open={deleteOpen} profile={deleting} ondeleted={() => list.reload()} />
{#if launchingFor}<LaunchDialog bind:open={launching} profileId={launchingFor} />{/if}

<div class="stack">
  <Tabs
    label="Profiles and templates"
    idPrefix="profiles"
    bind:value={() => tab, (next) => void setTab(next)}
    items={[
      { id: 'profiles', label: 'Profiles' },
      { id: 'templates', label: 'Templates' },
    ]}
  />

  {#if tab === 'templates'}
    <div role="tabpanel" id="profiles-panel-templates" aria-labelledby="profiles-tab-templates">
      <TemplatesTab />
    </div>
  {:else}
    <div role="tabpanel" id="profiles-panel-profiles" aria-labelledby="profiles-tab-profiles" class="stack">
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
        {@const resolved = resolveBuild(profile.settings.build, builds)}
        <td>
          <span class="name">
            {#if liveInstance && !liveInstance.queued}<StatusDot tone="success" label="Running" showLabel={false} />{/if}
            <a class="truncate" href="/profiles/{profile.id}">{profile.name}</a>
            {#if profile.pinned}<Badge>Pinned</Badge>{/if}
          </span>
        </td>
        <td>
          <span class="tags">
            {#each profile.tags.slice(0, 2) as tag (tag)}<Badge>{tag}</Badge>{/each}
            {#if profile.tags.length > 2}
              <span class="more" title={profile.tags.slice(2).join(', ')}>+{profile.tags.length - 2}</span>
            {/if}
            {#if profile.tags.length === 0}—{/if}
          </span>
        </td>
        <td>{profile.kind === 'persistent' ? 'Persistent' : 'Ephemeral'}</td>
        <td>
          {#if resolved.kind === 'build'}
            <span class="truncate" title={resolved.fallback ? 'No installed build matches this profile, so the browser detected on this computer is used.' : undefined}>{resolvedLabel(profile.settings.build, resolved)}</span>
          {:else if resolved.kind === 'missing'}
            <span class="warn"><Badge tone="warning">{missingLabel(resolved)}</Badge></span>
          {:else}
            <span class="warn"><Badge tone="warning">Pinned browser version is missing</Badge></span>
          {/if}
        </td>
        <td>{#if profile.login_status}<StateBadge kind="login" state={profile.login_status} />{:else}—{/if}</td>
        <td class="num right">
          <span class="disk">
            {#if profile.disk_quota_bytes != null && (usedBytes.get(profile.id) ?? 0) > profile.disk_quota_bytes}
              <span title="Launches are refused until usage drops below {bytes(profile.disk_quota_bytes)}"><Badge tone="danger">Over quota</Badge></span>
            {/if}
            {bytes(usedBytes.get(profile.id) ?? null)}
          </span>
        </td>
        <td>
          {#if profile.kind === 'persistent' && liveInstance?.queued}
            <Badge tone="warning">Waiting to start</Badge>
          {:else if profile.kind === 'persistent' && liveInstance}
            <Button size="sm" href="/instances/{liveInstance.id}">Open</Button>
          {:else}
            <Button size="sm" onclick={() => launch(profile)}>Launch</Button>
          {/if}
        </td>
        <td>
          <Menu
            label="Actions for {profile.name}"
            items={[
              { label: 'Edit', onselect: () => goto(`/profiles/${profile.id}`) },
              ...(profile.template_id
                ? [{ label: 'New profile from same template', onselect: () => openCreate(profile.template_id ?? undefined) }]
                : []),
              ...(!isBuiltin(profile)
                ? [
                    {
                      label: liveInstance ? 'Delete… (stop it first)' : 'Delete…',
                      disabled: Boolean(liveInstance),
                      hint: 'A running profile can\'t be deleted. Stop it first.',
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
            { key: 'name', label: 'Name', width: '20%' },
            { key: 'tags', label: 'Tags', width: '10%' },
            { key: 'kind', label: 'Kind', width: '11%' },
            { key: 'build', label: 'Browser', width: '31%' },
            { key: 'login', label: 'Login', width: '8%' },
            { key: 'disk', label: 'Disk', width: '10%', align: 'right' },
            { key: 'launch', label: 'Launch', width: '130px', srOnly: true },
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

  .warn {
    display: inline-flex;
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .filter {
    width: 280px;
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

  .tags {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-wrap: nowrap;
    overflow: hidden;
    white-space: nowrap;
  }

  .more {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
  }

  .disk {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    white-space: nowrap;
  }
</style>
