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
  import DetailLayout from '$lib/ui/DetailLayout.svelte';
  import Card from '$lib/ui/Card.svelte';
  import KeyValue from '$lib/ui/KeyValue.svelte';
  import RunTable from '$lib/runs/RunTable.svelte';
  import { absolute, bytes, trafficSummary } from '$lib/format';
  import SaveBar from '$lib/ui/SaveBar.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { guardUnsaved } from '$lib/ui/unsaved';
  import { toast } from '$lib/ui/toast.svelte';
  import type { components } from '$lib/api/schema';
  import SettingsForm from '$lib/config/SettingsForm.svelte';
  import GeneralCard from '$lib/config/GeneralCard.svelte';
  import { scrollToHash } from '$lib/scroll';
  import ExportProfileDialog from '$lib/profiles/ExportProfileDialog.svelte';
  import DeleteProfileDialog from '$lib/profiles/DeleteProfileDialog.svelte';
  import Backups from '$lib/config/sections/Backups.svelte';
  import Logins from '$lib/config/sections/Logins.svelte';
  import { isUnhealthy, proxyName, unhealthyReason } from '$lib/config/proxy';
  import { templateNameLabel } from '$lib/config/templates';
  import SaveAsTemplateDialog from '$lib/profiles/SaveAsTemplateDialog.svelte';
  import ReapplyTemplateDialog from '$lib/profiles/ReapplyTemplateDialog.svelte';
  import CookiesDialog from '$lib/profiles/CookiesDialog.svelte';
  import BookmarksDialog from '$lib/profiles/BookmarksDialog.svelte';
  import { canonical, engineOf, type GlobalSettings, type ProfileExtension, type Settings } from '$lib/config/form';
  import { settingsHref } from '$lib/settings/sections';

  type Profile = components['schemas']['Profile'];
  type PatchBody = Partial<components['schemas']['ProfileUpdateInput']>;

  const TRAFFIC_DAYS = 30;
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
  let reapplyOpen = $state(false);
  let deleteOpen = $state(false);
  let launchOpen = $state(false);
  let restarting = $state(false);
  let tab = $state('overview');
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
  const recentRuns = liveQuery(
    async () => (await unwrap(api.GET('/v1/instances', { params: { query: { profile: profileId, limit: 5 } } }))).instances,
    { kinds: ['instance.*'] },
  );
  const usageQuery = liveQuery(async () => (await unwrap(api.GET('/v1/storage/usage'))).profiles, {
    kinds: ['profile.*', 'artifact.*', 'retention.sweep_completed'],
  });
  const usedBytes = $derived((usageQuery.data ?? []).find((u) => u.profile_id === profileId)?.bytes ?? null);
  const trafficQuery = liveQuery(
    async () => (await unwrap(api.GET('/v1/usage/traffic', { params: { query: { group: 'profile', days: TRAFFIC_DAYS } } }))).items,
    { kinds: ['instance.state_changed'] },
  );
  const traffic = $derived((trafficQuery.data ?? []).find((item) => item.id === profileId));

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
        // Older notification links point at #logins or #backups.
        const hashTab = location.hash.slice(1);
        if (hashTab === 'logins' || hashTab === 'backups') tab = hashTab;
        else void scrollToHash();
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
      action: { label: 'Open Proxies', onclick: () => void goto(settingsHref('proxies')) },
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
  <DetailLayout
    title={current.name}
    description="{current.kind === 'persistent' ? 'Keeps data' : 'Fresh each time'} · {engine === 'camoufox' ? 'Camoufox' : 'Chromium'}"
    back={{ href: '/profiles', label: 'Profiles' }}
    tabs={[
      { id: 'overview', label: 'Overview' },
      { id: 'settings', label: 'Settings' },
      { id: 'logins', label: 'Logins & checks' },
      { id: 'cookies', label: 'Cookies & bookmarks' },
      { id: 'backups', label: 'Backups' },
    ]}
    bind:tab
  >
    {#snippet actions()}
      <Menu
        label="More actions"
        items={[
          {
            label: 'Re-apply template…',
            disabled: current.template_id == null,
            hint:
              current.template_id != null
                ? undefined
                : current.template_name
                  ? `The template ${templateNameLabel(current.template_name)} was deleted`
                  : 'This profile did not come from a template',
            onselect: () => (reapplyOpen = true),
          },
          { label: 'Save as template…', onselect: () => (tplOpen = true) },
          { label: 'Export profile…', disabled: running || current.kind !== 'persistent', hint: running ? 'Stop the profile first' : 'Only profiles that keep data can be exported', onselect: () => (exportOpen = true) },
          {
            label: 'Export cookies…',
            disabled: current.kind === 'ephemeral' && !running,
            hint: current.kind === 'ephemeral' && !running ? 'Fresh-each-time profiles keep no data while stopped; launch it to import or export cookies' : undefined,
            onselect: () => (cookiesExportOpen = true),
          },
          {
            label: 'Import cookies…',
            disabled: current.kind === 'ephemeral' && !running,
            hint: current.kind === 'ephemeral' && !running ? 'Fresh-each-time profiles keep no data while stopped; launch it to import or export cookies' : undefined,
            onselect: () => (cookiesImportOpen = true),
          },
          { label: 'Export bookmarks…', disabled: running || (current.kind === 'ephemeral' && !running), hint: running ? 'Stop the profile first' : current.kind === 'ephemeral' ? 'Fresh-each-time profiles keep no data while stopped; launch it to import or export cookies' : undefined, onselect: () => (bookmarksExportOpen = true) },
          { label: 'Import bookmarks…', disabled: running || (current.kind === 'ephemeral' && !running), hint: running ? 'Stop the profile first' : current.kind === 'ephemeral' ? 'Fresh-each-time profiles keep no data while stopped; launch it to import or export cookies' : undefined, onselect: () => (bookmarksImportOpen = true) },
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
        <Button variant="primary" href="/browsers/{active.id}">{queued ? 'Waiting to start' : 'Running'} · Open browser →</Button>
      {:else}
        <Button variant="primary" onclick={() => (launchOpen = true)}>Launch</Button>
      {/if}
    {/snippet}


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

  {#if tab === 'overview'}
    <div class="stack">
      <Card title="Overview">
        <KeyValue
          grid
          items={[
            { label: 'Kind', value: current.kind === 'persistent' ? 'Keeps data' : 'Fresh each time' },
            { label: 'Data folder', value: current.data_dir ?? null, mono: true, copy: Boolean(current.data_dir) },
            { label: 'Disk use', value: usedBytes === null ? null : bytes(usedBytes) },
            { label: 'Traffic (30 days)', value: traffic ? `${trafficSummary(traffic)} · ${traffic.runs} ${traffic.runs === 1 ? 'run' : 'runs'}` : trafficQuery.data ? 'None measured' : null },
            { label: 'Disk quota', value: current.disk_quota_bytes ? bytes(current.disk_quota_bytes) : 'No limit' },
            {
              label: 'Template',
              value: current.template_name
                ? current.template_id
                  ? templateNameLabel(current.template_name)
                  : `${templateNameLabel(current.template_name)} (deleted)`
                : null,
              href: current.template_id ? `/templates/${current.template_id}` : undefined,
            },
            { label: 'Created', value: absolute(current.created_at) },
          ]}
        />
        {#if current.disk_quota_bytes != null && usedBytes !== null && usedBytes > current.disk_quota_bytes}
          <p class="quota"><Badge tone="danger">Over quota</Badge> Launches are refused until usage drops below {bytes(current.disk_quota_bytes)}.</p>
        {/if}
      </Card>
      <Card title="Recent runs">
        <RunTable runs={recentRuns.data ?? []} loading={recentRuns.loading} error={recentRuns.error} compact hideProfile onretry={() => recentRuns.reload()} />
      </Card>
    </div>
  {:else if tab === 'logins'}
    <Logins profileId={current.id} />
  {:else if tab === 'backups'}
    <Backups profile={current} onsaved={(next) => (profile = next)} />
  {:else if tab === 'cookies'}
    <Card title="Cookies & bookmarks" description="Move cookies and bookmarks in or out of this profile.">
      <div class="actions-row">
        <Button disabled={current.kind === 'ephemeral' && !running} onclick={() => (cookiesExportOpen = true)}>Export cookies…</Button>
        <Button disabled={current.kind === 'ephemeral' && !running} onclick={() => (cookiesImportOpen = true)}>Import cookies…</Button>
        <Button disabled={running || (current.kind === 'ephemeral' && !running)} onclick={() => (bookmarksExportOpen = true)}>Export bookmarks…</Button>
        <Button disabled={running || (current.kind === 'ephemeral' && !running)} onclick={() => (bookmarksImportOpen = true)}>Import bookmarks…</Button>
      </div>
      {#if current.kind === 'ephemeral' && !running}
        <p class="field-hint">Fresh-each-time profiles keep no data while stopped; launch it to import or export cookies.</p>
      {:else if running}
        <p class="field-hint">Bookmarks can't move while the profile is running. Stop the profile first.</p>
      {/if}
    </Card>
  {:else}
    {#if current.template_name}
      <p class="banner">
        <Icon name="info" />
        Copied from Template {templateNameLabel(current.template_name)}{current.template_id ? '' : ' (since deleted)'} on {absolute(current.created_at)}. Changing the Template does not change this Profile.
      </p>
    {/if}
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
  </DetailLayout>

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
  <ReapplyTemplateDialog bind:open={reapplyOpen} profileId={current.id} onapplied={load} />
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

  .quota {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-3);
    color: var(--text-secondary);
  }

  .actions-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
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
