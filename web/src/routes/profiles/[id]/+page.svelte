<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { events } from '$lib/api/events.svelte';
  import { liveQuery } from '$lib/api/live.svelte';
  import { ENDED_STATES } from '$lib/state';
  import LaunchDialog from '$lib/instances/LaunchDialog.svelte';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import Tabs from '$lib/ui/Tabs.svelte';
  import { guardUnsaved } from '$lib/ui/unsaved';
  import { toast } from '$lib/ui/toast.svelte';
  import type { components } from '$lib/api/schema';
  import SettingsForm from '$lib/profiles/SettingsForm.svelte';
  import GeneralCard from '$lib/profiles/GeneralCard.svelte';
  import { scrollToHash } from '$lib/scroll';
  import ExportProfileDialog from '$lib/profiles/ExportProfileDialog.svelte';
  import DeleteProfileDialog from '$lib/profiles/DeleteProfileDialog.svelte';
  import Backups from '$lib/profiles/sections/Backups.svelte';
  import Logins from '$lib/profiles/sections/Logins.svelte';
  import { isUnhealthy, proxyName, unhealthyReason } from '$lib/profiles/proxy';
  import SaveAsTemplateDialog from '$lib/profiles/SaveAsTemplateDialog.svelte';
  import CookiesDialog from '$lib/profiles/CookiesDialog.svelte';
  import BookmarksDialog from '$lib/profiles/BookmarksDialog.svelte';
  import { canonical, engineOf, type GlobalSettings, type ProfileExtension, type Settings } from '$lib/profiles/form';

  type Profile = components['schemas']['Profile'];
  type PatchBody = Partial<components['schemas']['ProfileUpdateInput']>;

  const profileId = $derived(page.params.id ?? '');

  let profile = $state<Profile | null>(null);
  let loadError = $state<ApiError | null>(null);
  let loading = $state(true);
  let settings = $state<Settings | null>(null);
  let name = $state('');
  let tags = $state('');
  let notes = $state('');
  let pinned = $state(false);
  let quotaGb = $state<number | null>(null);
  let profileExtensions = $state<ProfileExtension[]>([]);
  let snapshot = $state('');
  let extSnapshot = $state('');
  let revision = $state(0);
  let saving = $state(false);
  let formInvalid = $state(false);
  let errors = $state<Record<string, string>>({});
  let changedElsewhere = $state(false);
  let extUnsupported = $state(false);
  let defaults = $state<GlobalSettings | undefined>(undefined);
  let builds = $state<components['schemas']['BuildView'][]>([]);
  let proxies = $state<components['schemas']['Proxy'][]>([]);
  let pools = $state<components['schemas']['Pool'][]>([]);
  let library = $state<components['schemas']['ExtensionView'][]>([]);
  let tplOpen = $state(false);
  let deleteOpen = $state(false);
  let launchOpen = $state(false);
  let restarting = $state(false);
  let tab = $state('settings');
  let exportOpen = $state(false);
  let cookiesExportOpen = $state(false);
  let cookiesImportOpen = $state(false);
  let bookmarksExportOpen = $state(false);
  let bookmarksImportOpen = $state(false);
  let anchored = false;

  const instances = liveQuery(
    async () => (await unwrap(api.GET('/v1/instances', { params: { query: { profile: profileId, limit: 100 } } }))).instances,
    { kinds: ['instance.*'] },
  );
  const active = $derived((instances.data ?? []).find((i) => !ENDED_STATES.includes(i.state)));
  const running = $derived(active !== undefined);
  const queued = $derived(active?.state === 'queued');

  // Restart now: once the browser the user stopped has ended, offer the Launch dialog.
  $effect(() => {
    if (restarting && !running) {
      restarting = false;
      launchOpen = true;
    }
  });

  async function restart() {
    if (!active) return;
    restarting = true;
    try {
      await unwrap(api.POST('/v1/instances/{instance_id}/stop', { params: { path: { instance_id: active.id } }, body: {} }));
    } catch (e) {
      restarting = false;
      toast.apiError(e, "Couldn't stop the browser");
    }
  }

  const currentSnapshot = $derived(settings ? canonical({ name, tags, notes, pinned, quotaGb, settings }) : '');
  const dirty = $derived(snapshot !== '' && (currentSnapshot !== snapshot || canonical(profileExtensions) !== extSnapshot));

  function applyProfile(next: Profile, extensions: ProfileExtension[]) {
    profile = next;
    settings = JSON.parse(JSON.stringify(next.settings)) as Settings;
    name = next.name;
    tags = next.tags.join(', ');
    notes = next.notes;
    pinned = next.pinned;
    quotaGb = next.disk_quota_bytes === null || next.disk_quota_bytes === undefined ? null : next.disk_quota_bytes / 1_000_000_000;
    profileExtensions = JSON.parse(JSON.stringify(extensions)) as ProfileExtension[];
    snapshot = canonical({ name, tags, notes, pinned, quotaGb, settings: next.settings });
    extSnapshot = canonical(extensions);
    changedElsewhere = false;
    extUnsupported = false;
    errors = {};
    revision += 1;
  }

  async function load() {
    loading = true;
    try {
      const [next, ext] = await Promise.all([
        unwrap(api.GET('/v1/profiles/{profile}', { params: { path: { profile: profileId } } })),
        unwrap(api.GET('/v1/profiles/{profile}/extensions', { params: { path: { profile: profileId } } })),
      ]);
      applyProfile(next, ext.extensions);
      loadError = null;
      if (!anchored) {
        anchored = true;
        // Notification links point at #logins or #backups.
        const hashTab = location.hash.slice(1);
        if (hashTab === 'logins' || hashTab === 'backups') tab = hashTab;
        void scrollToHash();
      }
    } catch (e) {
      loadError = e instanceof ApiError ? e : new ApiError(0, 'Unreachable', String(e), {});
    } finally {
      loading = false;
    }
  }

  async function loadProxies(): Promise<components['schemas']['Proxy'][]> {
    const all: components['schemas']['Proxy'][] = [];
    let cursor: string | null | undefined = undefined;
    for (let pageNo = 0; pageNo < 5; pageNo++) {
      const out: components['schemas']['ProxyListOutput'] = await unwrap(api.GET('/v1/proxies', { params: { query: { limit: 100, cursor } } }));
      all.push(...out.proxies);
      cursor = out.next_cursor;
      if (!cursor) break;
    }
    return all;
  }

  $effect(() => {
    void profileId;
    void load();
  });

  onMount(() => {
    void unwrap(api.GET('/v1/settings')).then((out) => (defaults = out.settings), (e) => toast.apiError(e, "Couldn't load settings"));
    void unwrap(api.GET('/v1/builds')).then((out) => (builds = out.builds), (e) => toast.apiError(e, "Couldn't load builds"));
    void loadProxies().then((all) => (proxies = all), (e) => toast.apiError(e, "Couldn't load proxies"));
    void unwrap(api.GET('/v1/proxy-pools')).then((out) => (pools = out.pools), (e) => toast.apiError(e, "Couldn't load proxy pools"));
    void unwrap(api.GET('/v1/extensions')).then((out) => (library = out.extensions), (e) => toast.apiError(e, "Couldn't load extensions"));

    const off = events.on(['profile.updated'], (e) => {
      if (e.subject.id !== profileId) return;
      if (untrack(() => dirty)) changedElsewhere = true;
      else void load();
    });
    return off;
  });

  const guard = guardUnsaved(() => dirty, 'Your edits to this profile will be lost.');

  // Saving a proxy that failed its health check is allowed, but the next launch would fail.
  function warnAboutProxy(before: Settings['proxy']) {
    const choice = settings?.proxy;
    if (!choice || !('proxy_id' in choice) || canonical(choice) === canonical(before)) return;
    const proxy = proxies.find((p) => p.id === choice.proxy_id);
    if (!proxy || !isUnhealthy(proxy)) return;
    toast.show({
      tone: 'warning',
      title: `${proxyName(proxy)} is unhealthy`,
      description: `${unhealthyReason(proxy)}. Launches with this proxy fail until it is fixed.`,
      action: { label: 'Open Proxies', onclick: () => void goto('/proxies') },
    });
  }

  async function save() {
    if (!profile || !settings || saving) return;
    saving = true;
    errors = {};
    extUnsupported = false;
    try {
      const before = JSON.parse(snapshot) as unknown as { name: string; tags: string; notes: string; pinned: boolean; quotaGb: number | null; settings: Settings };
      const body: PatchBody = { profile: profileId };
      if (name !== before.name) body.name = name;
      const tagList = tags.split(',').map((t) => t.trim()).filter(Boolean);
      if (canonical(tagList) !== canonical(before.tags.split(',').map((t) => t.trim()).filter(Boolean))) body.tags = tagList;
      if (notes !== before.notes) body.notes = notes;
      if (pinned !== before.pinned) body.pinned = pinned;
      body.disk_quota_bytes = quotaGb === null ? null : Math.round(quotaGb * 1_000_000_000);
      const profileChanged = currentSnapshot !== snapshot;
      if (canonical(settings) !== canonical(before.settings)) body.settings = settings;

      const extChanged = canonical(profileExtensions) !== extSnapshot;
      const oldIds = new Set((JSON.parse(extSnapshot || '[]') as ProfileExtension[]).map((e) => e.extension_id));
      const removalsOnly = profileExtensions.every((e) => oldIds.has(e.extension_id));

      const patchProfile = async () => {
        if (profileChanged) {
          profile = await unwrap(api.PATCH('/v1/profiles/{profile}', { params: { path: { profile: profileId } }, body }));
        }
      };
      const putExtensions = async () => {
        await unwrap(api.PUT('/v1/profiles/{profile}/extensions', { params: { path: { profile: profileId } }, body: { extensions: profileExtensions } }));
      };

      if (extChanged && removalsOnly) {
        await putExtensions();
        await patchProfile();
      } else {
        await patchProfile();
        if (extChanged) await putExtensions();
      }
      if (profile) applyProfile(profile, profileExtensions);
      if (running) {
        toast.show({
          tone: 'success',
          title: `Saved. ${name} ${queued ? 'is waiting to start' : 'is running'} — changes apply after ${queued ? 'it starts and restarts' : 'restart'}.`,
          action: { label: 'Restart now', onclick: () => void restart() },
        });
      } else {
        toast.success('Profile saved');
      }
      warnAboutProxy(before.settings.proxy);
    } catch (e) {
      if (e instanceof ApiError && e.code === 'EngineUnsupported' && e.details?.capability === 'unpacked_extensions') {
        extUnsupported = true;
        document.getElementById('extensions-title')?.scrollIntoView({ behavior: 'smooth' });
      } else if (e instanceof ApiError && e.code === 'InvalidArgument' && typeof e.details?.field === 'string') {
        errors = { [e.details.field]: e.message };
        await tick();
        if (!document.querySelector('[aria-invalid="true"]') && !errors['extensions'] && !errors['settings.extensions']) {
          toast.apiError(e, "Couldn't save the profile");
        } else {
          document.querySelector('[aria-invalid="true"]')?.scrollIntoView({ behavior: 'smooth', block: 'center' });
        }
      } else {
        toast.apiError(e, "Couldn't save the profile");
      }
    } finally {
      saving = false;
    }
  }

  const engine = $derived(settings ? engineOf(settings, builds) : 'chromium');
</script>

{#if loading && !profile}
  <div aria-busy="true" class="stack">
    <Skeleton width="240px" height="28px" />
    <Skeleton height="180px" radius="var(--radius-lg)" />
    <Skeleton height="180px" radius="var(--radius-lg)" />
  </div>
{:else if loadError?.code === 'NotFound' && !profile}
  <EmptyState level={1} title="Profile not found" description="It may have been deleted.">
    {#snippet action()}<Button href="/profiles">Back to profiles</Button>{/snippet}
  </EmptyState>
{:else if loadError && !profile}
  <EmptyState tone="danger" title="Couldn't load this profile" description={loadError.message}>
    {#snippet action()}<Button onclick={load}>Retry</Button>{/snippet}
  </EmptyState>
{:else if profile && settings}
  {@const current = profile}
  <PageHeader title={current.name} breadcrumb={[{ label: 'Profiles', href: '/profiles' }, { label: current.name, href: `/profiles/${current.id}` }]}>
    {#snippet badge()}
      <Badge>{current.kind === 'persistent' ? 'Persistent' : 'Ephemeral'}</Badge>
      <Badge>{engine === 'camoufox' ? 'Camoufox' : 'Chromium'}</Badge>
    {/snippet}
    {#snippet actions()}
      <Menu
        label="More actions"
        items={[
          { label: 'Save as template…', onselect: () => (tplOpen = true) },
          { label: 'Export profile…', disabled: running || current.kind !== 'persistent', hint: running ? 'Stop the profile first' : 'Only persistent profiles', onselect: () => (exportOpen = true) },
          { label: 'Export cookies…', onselect: () => (cookiesExportOpen = true) },
          { label: 'Import cookies…', onselect: () => (cookiesImportOpen = true) },
          { label: 'Export bookmarks…', disabled: running, hint: 'Stop the profile first', onselect: () => (bookmarksExportOpen = true) },
          { label: 'Import bookmarks…', disabled: running, hint: 'Stop the profile first', onselect: () => (bookmarksImportOpen = true) },
          ...(current.name === (defaults?.oneshot.profile ?? 'oneshot')
            ? []
            : [
                'separator' as const,
                {
                  label: running ? 'Delete… (stop it first)' : 'Delete…',
                  tone: 'danger' as const,
                  disabled: running,
                  hint: "A running profile can't be deleted. Stop it first.",
                  onselect: () => (deleteOpen = true),
                },
              ]),
        ]}
      >
        {#snippet trigger(props)}
          <Button variant="secondary" {...props}>More actions</Button>
        {/snippet}
      </Menu>
      {#if active?.state === 'queued'}
        <Badge tone="warning">Waiting to start</Badge>
      {:else if active}
        <Button variant="primary" href="/instances/{active.id}">{queued ? 'Waiting to start' : 'Running'} · Open browser →</Button>
      {:else}
        <Button variant="primary" onclick={() => (launchOpen = true)}>Launch</Button>
      {/if}
    {/snippet}
  </PageHeader>

  {#if running}
    <p class="banner">
      <Icon name="info" />
      {#if queued}{current.name} is waiting for a free slot. Changes apply after it starts and restarts.{:else}{current.name} is running. Changes apply after it restarts.{/if}
    </p>
  {/if}
  {#if changedElsewhere}
    <p class="banner">
      <Icon name="info" /> This profile changed elsewhere.
      <Button variant="ghost" size="sm" onclick={load}>Reload</Button>
    </p>
  {/if}

  <div class="tabs">
    <Tabs
      label="Profile sections"
      idPrefix="profile"
      bind:value={tab}
      items={[
        { id: 'settings', label: 'Settings' },
        { id: 'logins', label: 'Logins' },
        { id: 'backups', label: 'Backups' },
      ]}
    />
  </div>

  <div role="tabpanel" id="profile-panel-{tab}" aria-labelledby="profile-tab-{tab}">
    {#if tab === 'logins'}
      <Logins profileId={current.id} />
    {:else if tab === 'backups'}
      <Backups profile={current} onsaved={(next) => (profile = next)} />
    {:else}
  {#key revision}
    <SettingsForm
      bind:settings
      {errors}
      {builds}
      {proxies}
      {pools}
      {library}
      {defaults}
      {profileExtensions}
      onExtensions={(next) => (profileExtensions = next)}
      {extUnsupported}
      bind:pinned
      bind:quotaGb
      profileId={current.id}
      profileKind={current.kind}
      oninvalidchange={(invalid) => (formInvalid = invalid)}
    >
      {#snippet general()}
        <GeneralCard bind:name bind:tags bind:notes nameError={errors['name'] ?? null} tagsError={errors['tags'] ?? null} />
      {/snippet}
    </SettingsForm>
  {/key}
    {/if}
  </div>

  <SaveBar dirty={dirty} {saving} disabled={formInvalid} onsave={save} ondiscard={load} />

  <DeleteProfileDialog
    bind:open={deleteOpen}
    profile={current}
    ondeleted={() => {
      guard.allow();
      void goto('/profiles');
    }}
  />
  {#if launchOpen}<LaunchDialog bind:open={launchOpen} profileId={current.id} />{/if}
  <ExportProfileDialog bind:open={exportOpen} profileId={current.id} profileName={current.name} />
  <SaveAsTemplateDialog bind:open={tplOpen} profileId={current.id} profileName={current.name} />
  <CookiesDialog bind:open={cookiesExportOpen} mode="export" profileId={current.id} profileName={current.name} />
  <CookiesDialog bind:open={cookiesImportOpen} mode="import" profileId={current.id} profileName={current.name} />
  <BookmarksDialog bind:open={bookmarksExportOpen} mode="export" profileId={current.id} profileName={current.name} />
  <BookmarksDialog bind:open={bookmarksImportOpen} mode="import" profileId={current.id} profileName={current.name} />
{/if}

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .tabs {
    margin-bottom: var(--space-4);
  }

  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-4);
    padding: var(--space-3) var(--space-4);
    background: var(--bg-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    color: var(--text-secondary);
  }
</style>
