<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { events } from '$lib/api/events.svelte';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components } from '$lib/api/schema';
  import { attention, hostOf, REASON_VERBS } from '$lib/attention/attention.svelte';
  import { detectBrowsers } from '$lib/builds/detect';
  import { capacity } from '$lib/capacity.svelte';
  import { clock, everySecond } from '$lib/clock.svelte';
  import { persistedKinds } from '$lib/events/kinds';
  import { countdown, endReason, plainError } from '$lib/events/summary';
  import { bytes, duration, relative } from '$lib/format';
  import { outcome, profileLabel } from '$lib/instances/actions';
  import LaunchDialog from '$lib/instances/LaunchDialog.svelte';
  import Timeline from '$lib/instances/Timeline.svelte';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import Progress from '$lib/ui/Progress.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Build = components['schemas']['BuildView'];
  type Tone = 'success' | 'warning' | 'danger';

  const RECENT = 5;
  const RECENT_KINDS = (['browsers', 'tabs', 'setup'] as const).flatMap(persistedKinds).join(',');
  const RUNNING_STATES = ['starting', 'running', 'idle', 'reaping', 'attaching', 'attached'];

  const builds = liveQuery(async () => (await unwrap(api.GET('/v1/builds'))).builds, {
    kinds: ['build.installed', 'build.install_failed', 'build.removed'],
  });
  const profiles = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, { kinds: ['profile.created', 'profile.deleted'] });
  const recent = liveQuery(async () => (await unwrap(api.GET('/v1/events', { params: { query: { kinds: RECENT_KINDS, order: 'desc', limit: RECENT } } }))).events, {
    kinds: RECENT_KINDS.split(','),
  });

  let launchOpen = $state(false);
  let now = $state(Date.now());
  $effect(() => {
    if (!attention.needsYou.length) return;

    return everySecond((tick) => (now = tick));
  });

  const chromium = (b: Build) => b.engine === 'chromium';
  const hasBrowser = $derived(builds.data?.some((b) => chromium(b) && ['installed', 'registered'].includes(b.state)));
  const installing = $derived(builds.data?.find((b) => chromium(b) && b.state === 'installing'));
  const hasProfile = $derived(profiles.data?.some((p) => !p.builtin));
  const showSteps = $derived(!hasBrowser || !hasProfile);

  let detecting = $state(false);
  let installId = $state<string | null>(null);
  let notFound = $state(false);
  let installError = $state<string | null>(null);
  let progress = $state<{ bytes: number; total: number | null; extracting: boolean } | null>(null);

  const watchedId = $derived(installId ?? installing?.id ?? null);
  const showProgress = $derived(installId !== null || installing !== undefined);

  $effect(() => {
    const offProgress = events.on(['build.install_progress'], (e) => {
      const data = e.data as { build_id: string; bytes: number; total?: number | null; phase: 'downloading' | 'extracting' };
      if (data.build_id === watchedId) progress = { bytes: data.bytes, total: data.total ?? null, extracting: data.phase === 'extracting' };
    });
    const offFailed = events.on(['build.install_failed'], (e) => {
      const data = e.data as { build_id: string; error?: string };
      if (data.build_id !== watchedId) return;
      installError = data.error ?? 'The install failed.';
      installId = null;
      progress = null;
    });
    const offInstalled = events.on(['build.installed'], (e) => {
      if ((e.data as { build_id: string }).build_id === watchedId) installId = null;
    });
    return () => {
      offProgress();
      offFailed();
      offInstalled();
    };
  });

  async function detect() {
    detecting = true;
    notFound = false;
    const found = await detectBrowsers(builds.data);
    if (found) notFound = !found.some((b) => b.engine === 'chromium');
    builds.reload();
    detecting = false;
  }

  async function install() {
    installError = null;
    progress = null;
    try {
      installId = (await unwrap(api.POST('/v1/builds', { body: { source: 'cft', channel: 'stable' } }))).id;
      builds.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't start the install");
    }
  }

  const running = $derived((capacity.usage?.instances ?? []).filter((i) => RUNNING_STATES.includes(i.state)));
  // Who holds each browser: the agents' labels, or "You're in control" when this UI does.
  const holders = (instanceId: string): string => {
    const labels = (capacity.usage?.leases ?? [])
      .filter((l) => l.instance_id === instanceId)
      .map((l) => (l.client_kind === 'ui' ? "You're in control" : l.client_label));
    return [...new Set(labels)].join(', ') || '—';
  };

  type Alert = { key: string; tone: Tone; title: string; detail: string; href: string; action: string; dismissible?: boolean };
  // A person waiting on you resolves itself; failures and unhealthy proxies stay until dismissed.
  const alerts = $derived<Alert[]>([
    ...attention.needsYou.map((n) => ({
      key: `tab:${n.tabId}`,
      tone: 'warning' as const,
      title: `${n.agent} needs you to ${REASON_VERBS[n.reason]} on ${hostOf(n.url)}`,
      detail: countdown(n.deadline, now),
      href: `/instances/${n.instanceId}?tab=${n.tabId}`,
      action: 'Step in',
    })),
    ...[
      ...attention.failures.map((i) => ({
        key: `instance:${i.id}`,
        tone: 'danger' as const,
        title: `${profileLabel(i)} ${outcome(i)?.label === 'Crashed' ? 'crashed' : "couldn't start"}`,
        detail: `${relative(i.ended_at ?? i.last_activity_at, clock.now)} · ${i.error ? plainError(i.error) : i.end_reason ? endReason(i.end_reason) : 'No reason recorded'}`,
        href: `/instances/${i.id}`,
        action: 'View',
      })),
      ...attention.unhealthyProxies.map((p) => ({
        key: `proxy:${p.id}:${p.checked_at}`,
        tone: 'warning' as const,
        title: `Proxy ${p.scheme}://${p.host}:${p.port} is unhealthy`,
        detail: p.last_error ?? 'Its last health check failed',
        href: '/proxies',
        action: 'Open',
      })),
    ]
      .filter((alert) => !attention.dismissed.has(alert.key))
      .map((alert) => ({ ...alert, dismissible: true })),
  ]);

  const toneFor = (ratio: number): Tone => (ratio >= 1 ? 'danger' : ratio >= 0.75 ? 'warning' : 'success');
  const mb = (n: number) => bytes(n * 1048576);

  const meters = $derived.by(() => {
    const { info, limits } = capacity;
    if (!info || !limits) return [];

    const fixed = limits.mode !== 'adaptive';
    const used = info.mem_total_mb - info.mem_available_mb;
    const floor = limits.adaptive.min_free_ram_mb;
    const load = info.load_per_core / limits.adaptive.max_load_per_core;
    const tabs = capacity.tabs / Math.max(1, limits.max_tabs_total);
    const browsers = capacity.browsers / Math.max(1, limits.max_instances);
    return [
      {
        label: 'Browsers running',
        text: `${capacity.browsers} of ${limits.max_instances} allowed`,
        value: Math.min(100, browsers * 100),
        tone: toneFor(browsers),
      },
      {
        label: 'Tabs open',
        text: `${capacity.tabs} of ${limits.max_tabs_total} allowed`,
        value: Math.min(100, tabs * 100),
        tone: toneFor(tabs),
      },
      {
        label: 'Memory used',
        text: `${mb(used)} of ${mb(info.mem_total_mb)}${fixed ? ' · not enforced' : ` · brr keeps ${mb(floor)} free`}`,
        value: (used / Math.max(1, info.mem_total_mb)) * 100,
        tone: fixed ? ('accent' as const) : toneFor(used / Math.max(1, info.mem_total_mb - floor)),
      },
      {
        label: 'CPU busy',
        text: `${info.load_per_core.toFixed(2)} per core${fixed ? ' · not enforced' : ` · pauses new browsers above ${limits.adaptive.max_load_per_core}`}`,
        value: Math.min(100, load * 100),
        tone: fixed ? ('accent' as const) : toneFor(load),
      },
    ];
  });

  const runningColumns = [
    { key: 'profile', label: 'Profile', width: '34%' },
    { key: 'holder', label: 'In use by' },
    { key: 'tabs', label: 'Tabs', width: '56px', align: 'right' as const },
    { key: 'open', label: 'Open', srOnly: true, keepWidth: true, width: '96px' },
  ];
  const queueColumns = [
    { key: 'profile', label: 'Profile' },
    { key: 'waiting', label: 'Waiting', width: '96px' },
    { key: 'cancel', label: 'Cancel', srOnly: true, keepWidth: true, width: '88px' },
  ];

  // Stopping a queued launch cancels it.
  async function cancelLaunch(instanceId: string, name: string) {
    try {
      await unwrap(api.POST('/v1/instances/{instance_id}/stop', { params: { path: { instance_id: instanceId } }, body: {} }));
      toast.success(`Cancelled the launch of ${name}`);
    } catch (e) {
      toast.apiError(e, "Couldn't cancel the launch");
    }
  }
</script>

<PageHeader title="Dashboard">
  {#snippet actions()}
    {#if hasBrowser}<Button variant="primary" icon="play" onclick={() => (launchOpen = true)}>Launch browser</Button>{/if}
  {/snippet}
</PageHeader>

{#if builds.loading || profiles.loading}
  <div class="stack" aria-busy="true">
    <Skeleton height="160px" radius="var(--radius-lg)" />
    <Skeleton height="160px" radius="var(--radius-lg)" />
  </div>
{:else if builds.error && !builds.data}
  <Card>
    <EmptyState tone="danger" title="Couldn't load browser versions" description={builds.error.message}>
      {#snippet action()}<Button onclick={() => builds.reload()}>Retry</Button>{/snippet}
    </EmptyState>
  </Card>
{:else}
  <div class="stack">
    {#if showSteps}
      <Card title="Get started" description="brr runs real browsers for your AI agents and lets you watch and step in.">
        <ol class="steps">
          <li class:done={hasBrowser}>
            <span class="num-badge num" aria-hidden="true">1</span>
            <div class="step-body">
              <h3>Add a browser version</h3>
              {#if hasBrowser}
                <p>Done. A browser version is installed.</p>
              {:else}
                <p>Use the Chrome already on this machine, or install Chrome for Testing.</p>
                {#if showProgress}
                  <div class="install">
                    <Progress
                      label="Installing Chrome for Testing"
                      size="md"
                      value={progress?.total && !progress.extracting ? (progress.bytes / progress.total) * 100 : null}
                    />
                    <p class="note num">
                      {#if progress?.extracting}Extracting…{:else if progress}{bytes(progress.bytes)}{progress.total ? ` of ${bytes(progress.total)}` : ''}{:else}Starting the download{/if}
                    </p>
                  </div>
                {:else}
                  <div class="actions">
                    <Button variant="primary" loading={detecting} onclick={detect}>Use system Chrome</Button>
                    <Button onclick={install}>{installError ? 'Retry' : 'Install Chrome for Testing'}</Button>
                  </div>
                  {#if notFound}<p class="error">No system Chrome, Chromium, Brave or Edge found.</p>{/if}
                  {#if installError}<p class="error">{installError}</p>{/if}
                {/if}
              {/if}
            </div>
          </li>
          <li class:done={hasProfile}>
            <span class="num-badge num" aria-hidden="true">2</span>
            <div class="step-body">
              <h3>Create a profile</h3>
              <p>{hasProfile ? 'Done. You have a profile.' : 'A profile is a browser identity: its settings, logins and cookies.'}</p>
              {#if hasBrowser && !hasProfile}<div class="actions"><Button variant="primary" href="/profiles">Create a profile</Button></div>{/if}
            </div>
          </li>
          <li>
            <span class="num-badge num" aria-hidden="true">3</span>
            <div class="step-body">
              <h3>Connect an agent</h3>
              <p>Point Claude Code, Cursor, Codex or your own code at brr.</p>
              <div class="actions"><Button href="/connect">Connect an agent</Button></div>
            </div>
          </li>
        </ol>
      </Card>
    {/if}

    {#if hasBrowser}
      <div class="below">
        <div class="col">
          <Card title="Needs attention">
            {#if attention.loading}
              <Skeleton width="60%" />
            {:else if attention.error && !alerts.length}
              <EmptyState compact title="Couldn't check what needs you. Retrying when the daemon answers." />
            {:else if alerts.length}
              <ul class="alerts">
                {#each alerts as alert, index (alert.key)}
                  <li>
                    <div class="who">
                      <p class="title">{alert.title}</p>
                      <p class="meta"><Badge tone={alert.tone}>{alert.tone === 'danger' ? 'Failed' : 'Needs you'}</Badge><span class="num">{alert.detail}</span></p>
                    </div>
                    <div class="row-actions">
                      {#if alert.dismissible}<Button variant="ghost" size="sm" aria-label="Dismiss: {alert.title}" onclick={() => attention.dismiss(alert.key)}>Dismiss</Button>{/if}
                      <Button variant={index === 0 && alert.tone === 'warning' ? 'primary' : 'secondary'} size="sm" href={alert.href}>{alert.action}</Button>
                    </div>
                  </li>
                {/each}
              </ul>
            {:else}
              <EmptyState compact title="Nothing needs you right now." />
            {/if}
          </Card>

          <Card title="Running now" flush>
            <Table
              caption="Running browsers"
              columns={runningColumns}
              rows={running}
              key={(row) => row.id}
              loading={capacity.loading}
              error={capacity.error}
              onretry={() => capacity.reload()}
              empty={{ compact: true, title: 'No browsers running. Launch one, or connect an agent.' }}
              flush
            >
              {#snippet row(instance)}
                <td class="truncate" title={profileLabel(instance)}>{profileLabel(instance)}</td>
                <td class="truncate" title={holders(instance.id)}>{holders(instance.id)}</td>
                <td class="num right">{instance.tab_count}</td>
                <td><Button size="sm" href="/instances/{instance.id}" aria-label="Open {profileLabel(instance)}">Open</Button></td>
              {/snippet}
            </Table>
          </Card>

          {#if capacity.queue?.length}
            <Card title="Waiting to start" flush>
              <Table
                caption="Browsers waiting to start"
                columns={queueColumns}
                rows={capacity.queue}
                key={(row) => row.instance_id}
                loading={capacity.loading}
                error={capacity.error}
                onretry={() => capacity.reload()}
                empty={{ compact: true, title: 'Nothing is waiting to start.' }}
                flush
              >
                {#snippet row(entry)}
                  <td class="truncate" title={entry.profile_name}>{entry.profile_name}</td>
                  <td class="num">{duration(Math.max(0, clock.now - entry.queued_at))}</td>
                  <td><Button size="sm" onclick={() => cancelLaunch(entry.instance_id, entry.profile_name)} aria-label="Cancel {entry.profile_name}">Cancel</Button></td>
                {/snippet}
              </Table>
            </Card>
          {/if}
        </div>

        <div class="col">
          <Card title="Capacity">
            {#snippet actions()}
              <Badge>{capacity.limits?.mode === 'adaptive' ? 'Based on free memory' : 'Fixed limits'}</Badge>
              <Button variant="ghost" size="sm" href="/limits">Change</Button>
            {/snippet}
            {#if capacity.error && !capacity.info}
              <EmptyState tone="danger" title="Couldn't load capacity" description={capacity.error.message}>
                {#snippet action()}<Button onclick={() => capacity.reload()}>Retry</Button>{/snippet}
              </EmptyState>
            {:else if capacity.loading}
              <div class="meters" aria-busy="true">
                {#each { length: 4 } as _, i (i)}<Skeleton height="32px" radius="var(--radius-md)" />{/each}
              </div>
            {:else}
              <dl class="meters">
                {#each meters as row (row.label)}
                  <div class="meter">
                    <dt>{row.label}</dt>
                    <dd class="num">{row.text}</dd>
                    <dd class="bar"><Progress label={row.label} value={row.value} tone={row.tone} size="md" /></dd>
                  </div>
                {/each}
              </dl>
            {/if}
          </Card>

          <Card title="Recent activity">
            {#snippet actions()}<Button variant="ghost" size="sm" href="/events">View all</Button>{/snippet}
            <Timeline events={recent.data} loading={recent.loading} error={recent.error} onretry={() => recent.reload()} />
          </Card>
        </div>
      </div>
    {/if}
  </div>
{/if}

<LaunchDialog bind:open={launchOpen} />

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  /* Two independent columns, so a short card never leaves a hole beside a tall one. */
  .below {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-4);
    align-items: start;
  }

  .col {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  @media (max-width: 899px) {
    .below {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .steps {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .steps li {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: var(--space-3);
  }

  .num-badge {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-full);
    background: var(--accent);
    color: var(--accent-fg);
    font-size: var(--text-sm-size);
    font-weight: 600;
  }

  .done .num-badge {
    background: var(--bg-muted);
    color: var(--text-secondary);
  }

  h3 {
    font-size: var(--text-md-size);
    line-height: var(--text-md-line);
    letter-spacing: 0;
  }

  .step-body p {
    margin-top: var(--space-1);
    color: var(--text-secondary);
  }

  .actions {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }

  .install {
    max-width: 320px;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }

  .note {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .error {
    margin-top: var(--space-3);
    color: var(--danger-fg);
  }

  .alerts {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .alerts li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) 0;
  }

  .title {
    font-weight: 500;
  }

  .row-actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .meters {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    margin: 0;
  }

  .meter {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--space-2) var(--space-4);
  }

  .bar {
    grid-column: 1 / -1;
  }

  dt {
    font-weight: 500;
  }

  dd {
    margin: 0;
    color: var(--text-secondary);
  }
</style>
