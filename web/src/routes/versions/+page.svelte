<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { events } from '$lib/api/events.svelte';
  import { liveQuery } from '$lib/api/live.svelte';
  import AvailableBuilds from '$lib/builds/AvailableBuilds.svelte';
  import { detectBrowsers } from '$lib/builds/detect';
  import InstallDialog from '$lib/builds/InstallDialog.svelte';
  import { buildLabel, INSTALLABLE_SOURCES, SOURCE_NAMES, type BuildSource } from '$lib/builds/labels';
  import { resolveBuild } from '$lib/config/resolve';
  import { clock } from '$lib/clock.svelte';
  import { bytes, relative } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import Progress from '$lib/ui/Progress.svelte';
  import Select from '$lib/ui/Select.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';
  import Table from '$lib/ui/Table.svelte';
  import Tabs from '$lib/ui/Tabs.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import type { components } from '$lib/api/schema';

  type BuildView = components['schemas']['BuildView'];

  type Capability = components['schemas']['Capability'];

  const CAPABILITY_LABELS: Record<Capability, string> = {
    pdf: 'PDF',
    mhtml: 'MHTML',
    screencast: 'Screencast',
    unpacked_extensions: 'Extensions',
    har: 'HAR',
    request_interception: 'Interception',
    fingerprint_config: 'Fingerprint',
  };
  // What every Chromium build offers; only deviations from it get a chip of their own.
  const STANDARD: Capability[] = ['pdf', 'mhtml', 'screencast', 'unpacked_extensions', 'har', 'request_interception'];

  // One line: "Standard", then only what deviates from it ("Standard · no PDF", "Fingerprint").
  function capabilitySummary(build: BuildView): { text: string; title: string } {
    const has = (cap: Capability) => build.capabilities.includes(cap);
    const missing = STANDARD.filter((cap) => !has(cap));
    const all = build.capabilities.map((cap) => CAPABILITY_LABELS[cap]).join(', ');
    if (missing.length > 2) return { text: all, title: all };
    const extra = build.capabilities.filter((cap) => !STANDARD.includes(cap)).map((cap) => CAPABILITY_LABELS[cap]);
    const text = ['Standard', ...missing.map((cap) => `no ${CAPABILITY_LABELS[cap]}`), ...extra].join(' · ');
    return { text, title: all };
  }

  const BUILD_STATE_LABELS: Record<string, string> = { registered: 'Detected', installed: 'Installed' };

  const isSource = (value: string | null): value is BuildSource => value !== null && (INSTALLABLE_SOURCES as string[]).includes(value);

  let tab = $state(page.url.searchParams.get('tab') === 'available' ? 'available' : 'installed');
  let source = $state<BuildSource>(isSource(page.url.searchParams.get('source')) ? (page.url.searchParams.get('source') as BuildSource) : 'cft');
  let installingOpen = $state(false);
  let detecting = $state(false);
  let progress = $state<Record<string, { bytes: number; total: number | null; extracting: boolean }>>({});
  // Installs started elsewhere (the CLI) have no row yet; the first progress event for one reloads the list.
  const asked = new Set<string>();

  const profiles = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, { kinds: ['profile.*', 'build.*'] });
  const templatesQuery = liveQuery(async () => (await unwrap(api.GET('/v1/templates'))).templates, { kinds: ['template.*', 'build.*'] });

  const list = liveQuery(async () => (await unwrap(api.GET('/v1/builds'))).builds, {
    kinds: ['build.installed', 'build.install_failed', 'build.removed', 'build.channel_moved'],
  });

  onMount(() =>
    events.on(['build.install_progress', 'build.install_failed'], (e) => {
      if (e.kind === 'build.install_progress') {
        const data = e.data as { build_id: string; bytes: number; total?: number | null; phase: 'downloading' | 'extracting' };
        progress = { ...progress, [data.build_id]: { bytes: data.bytes, total: data.total ?? null, extracting: data.phase === 'extracting' } };
        if (!asked.has(data.build_id) && !list.data?.some((b) => b.id === data.build_id)) {
          asked.add(data.build_id);
          list.reload();
        }
      } else if (e.kind === 'build.install_failed') {
        const data = e.data as { error?: string };
        if (data.error) toast.error('Browser version install failed', data.error);
      }
    }),
  );

  $effect(() => {
    const params = new URLSearchParams();
    if (tab === 'available') {
      params.set('tab', 'available');
      params.set('source', source);
    }
    const query = params.toString();
    void goto(query ? `/versions?${query}` : '/versions', { replaceState: true, keepFocus: true, noScroll: true });
  });

  // Installing builds first, then newest; a finished install keeps its row.
  const sorted = $derived(
    list.data?.toSorted(
      (a, b) =>
        Number(b.state === 'installing') - Number(a.state === 'installing') ||
        (b.installed_at ?? 0) - (a.installed_at ?? 0) ||
        a.id.localeCompare(b.id),
    ),
  );

  const installedKeys = $derived(new Set((list.data ?? []).map((b) => `${b.source}:${b.version}`)));

  // A channel reference is labelled "cft:stable" or "camoufox:latest-compatible"; show only the channel, in words.
  function channelName(label: string): string {
    const name = label.slice(label.indexOf(':') + 1).replace(/-/g, ' ');
    return name[0].toUpperCase() + name.slice(1);
  }

  const plural = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;

  const referencesOf = (build: BuildView, kind: BuildView['referenced_by'][number]['kind']) => build.referenced_by.filter((r) => r.kind === kind);

  type Usage = { channels: string[]; templates: string[]; profiles: string[]; runs: number };

  // Pinned or channel-resolved: a Template or Profile uses the build when its choice resolves to it,
  // such as Chrome for Testing falling back to a detected system browser.
  function usage(build: BuildView): Usage {
    const all = list.data ?? [];
    const resolves = (choice: components['schemas']['ProfileSettings']['build']) => {
      const found = resolveBuild(choice, all);
      return found.kind === 'build' && found.build.id === build.id;
    };
    const templates = (templatesQuery.data ?? []).filter((t) => resolves(t.settings.build)).map((t) => t.name);
    const profileNames = (profiles.data ?? []).filter((p) => !p.builtin && resolves(p.settings.build)).map((p) => p.name);
    return {
      channels: referencesOf(build, 'channel').map((r) => `${channelName(r.label)} channel`),
      templates: [...new Set(templates)],
      profiles: [...new Set([...referencesOf(build, 'profile').map((r) => r.label), ...profileNames])],
      runs: referencesOf(build, 'instance').length,
    };
  }

  const usersCount = (u: Usage) => u.channels.length + u.templates.length + u.profiles.length + u.runs;
  const inUse = (u: Usage) => u.channels.length + u.templates.length + u.profiles.length + u.runs > 0;

  // "Stable channel · 2 Templates · 1 Profile · 1 running browser"
  function usedByText(u: Usage): string {
    const parts = [...u.channels];
    if (u.templates.length) parts.push(plural(u.templates.length, 'Template'));
    if (u.profiles.length) parts.push(plural(u.profiles.length, 'Profile'));
    if (u.runs) parts.push(plural(u.runs, 'running browser'));
    return parts.join(' · ') || '—';
  }

  // Full lists, for the tooltip and the Remove dialog.
  function usedByDetail(u: Usage): string {
    const parts = [...u.channels];
    if (u.templates.length) parts.push(`${u.templates.length === 1 ? 'Template' : 'Templates'}: ${u.templates.join(', ')}`);
    if (u.profiles.length) parts.push(`${u.profiles.length === 1 ? 'Profile' : 'Profiles'}: ${u.profiles.join(', ')}`);
    if (u.runs) parts.push(plural(u.runs, 'running browser'));
    return parts.join('; ') || 'Not used';
  }

  const ENGINE_NAMES = { chromium: 'Chromium', camoufox: 'Camoufox' } as const;

  async function remove(build: BuildView) {
    if (build.source === 'system') {
      await confirm({
        title: `${buildLabel(build)} can't be removed here`,
        description: "brr found this browser on your computer but didn't install it, so it can't delete it. Uninstall it with your system's package manager to remove it.",
        confirmLabel: 'OK',
        info: true,
      });
      return;
    }
    const u = usage(build);
    if (inUse(u)) {
      await confirm({
        title: `${buildLabel(build)} is in use`,
        description: `It can't be removed while it is used by ${usedByDetail(u)}. Switch ${usersCount(u) === 1 ? 'it' : 'them'} to another browser version first.`,
        confirmLabel: 'OK',
        info: true,
      });
      return;
    }
    const ok = await confirm({
      title: `Remove ${buildLabel(build)}?`,
      description: `No Template, Profile or running browser uses it. Removing it deletes its files and frees ${bytes(build.size_bytes)}.`,
      confirmLabel: 'Remove browser version',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await unwrap(api.DELETE('/v1/builds/{build_id}', { params: { path: { build_id: build.id } } }));
      toast.success('Browser version removed');
    } catch (e) {
      // The API also refuses while in use and its message names what uses the version.
      toast.apiError(e, "Couldn't remove the browser version");
    }
  }

  async function detect() {
    detecting = true;
    await detectBrowsers(list.data);
    list.reload();
    detecting = false;
  }
</script>

<PageHeader title="Browser versions" description="The browsers brr can launch. Templates and Profiles pick one of these.">
  {#snippet actions()}
    <Button loading={detecting} onclick={detect}>Detect installed browsers</Button>
    <Button variant="primary" icon="plus" onclick={() => (installingOpen = true)}>Install</Button>
  {/snippet}
</PageHeader>

<InstallDialog bind:open={installingOpen} oninstalled={() => { tab = 'installed'; list.reload(); }} />

<div class="stack">
  <Tabs
    items={[
      { id: 'installed', label: 'Installed' },
      { id: 'available', label: 'Available' },
    ]}
    bind:value={tab}
    label="Browser versions"
    idPrefix="builds"
  />

  {#if tab === 'installed'}
    <div role="tabpanel" id="builds-panel-installed" aria-labelledby="builds-tab-installed">
      <p class="hint">Capabilities: “Standard” means PDF, MHTML, screencast, unpacked extensions, HAR and request interception all work; only differences are listed.</p>
      <Table
        caption="Installed browser versions"
        columns={[
          { key: 'build', label: 'Version', width: '19%' },
          { key: 'engine', label: 'Engine', width: '11%' },
          { key: 'state', label: 'State', width: '15%' },
          { key: 'capabilities', label: 'Capabilities', width: '14%' },
          { key: 'used', label: 'Used by', width: '23%' },
          { key: 'size', label: 'Size', align: 'right', width: '8%' },
          { key: 'installed', label: 'Installed', width: '10%' },
          { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
        ]}
        rows={sorted}
        key={(row: BuildView) => row.id}
        loading={list.loading}
        error={list.error}
        onretry={() => list.reload()}
        empty={{ icon: 'package', title: 'No browser versions yet', description: 'Detect the browsers on this computer or install Chrome for Testing.' }}
      >
        {#snippet row(build: BuildView)}
          <td class="wrap">
            <span title={build.source === 'system' ? `${buildLabel(build)} (detected on this computer)` : buildLabel(build)}>{buildLabel(build)}</span>
            {#if build.channel}<span class="sub">{channelName(build.channel)} channel</span>{/if}
          </td>
          <td class="nowrap">{ENGINE_NAMES[build.engine]}</td>
          <td>
            {#if build.state === 'installing'}
              {@const p = progress[build.id]}
              {@const extracting = p?.extracting === true}
              <div class="progress-cell">
                <Progress value={p?.total && !extracting ? Math.round((p.bytes / p.total) * 100) : null} label="Installing {buildLabel(build)}" />
                <span class="num">{extracting ? 'Extracting…' : p?.total ? `${Math.round((p.bytes / p.total) * 100)}%` : p ? bytes(p.bytes) : 'Starting…'}</span>
              </div>
            {:else}
              <StateBadge kind="build" state={build.state} label={BUILD_STATE_LABELS[build.state]} />
            {/if}
          </td>
          {@const caps = capabilitySummary(build)}
          {@const used = usage(build)}
          <td class="wrap" title={caps.title}>{caps.text}</td>
          <td class="wrap" title={usedByDetail(used)}>{usedByText(used)}</td>
          <td class="num right nowrap">{build.source === 'system' ? '—' : bytes(build.size_bytes)}</td>
          <td class="nowrap">{build.installed_at ? relative(build.installed_at, clock.now) : '—'}</td>
          <td><Menu label="Actions for {buildLabel(build)}" items={[{ label: 'Remove…', tone: 'danger', onselect: () => void remove(build) }]} /></td>
        {/snippet}
      </Table>
    </div>
  {:else}
    <div role="tabpanel" id="builds-panel-available" aria-labelledby="builds-tab-available" class="stack">
      <div class="source-row">
        <Select label="Source" hideLabel bind:value={source} options={INSTALLABLE_SOURCES.map((s) => ({ value: s, label: SOURCE_NAMES[s] }))} />
      </div>
      {#key source}
        <AvailableBuilds {source} {installedKeys} oninstall={() => { tab = 'installed'; list.reload(); }} />
      {/key}
    </div>
  {/if}
</div>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .nowrap {
    white-space: nowrap;
  }

  .wrap {
    white-space: normal;
    overflow-wrap: anywhere;
    padding-block: var(--space-2);
  }

  .sub {
    display: block;
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
  }

  .hint {
    margin: 0 0 var(--space-3);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
  }

  .progress-cell {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .source-row {
    max-width: 320px;
  }
</style>
