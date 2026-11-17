<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import Badge from '$lib/ui/Badge.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import { canonical, type SectionProps, type Settings } from './form';
  import { ADVANCED_NOTE, ADVANCED_SECTIONS, BASIC_SECTIONS } from './sections';

  const SECTION_FIELDS: Record<string, (keyof Settings)[]> = {
    version: ['build'],
    display: ['display', 'viewport', 'screen'],
    network: ['proxy'],
    locale: ['locale', 'timezone'],
    'launch-options': ['args', 'env', 'prefs', 'user_agent'],
    'network-privacy': ['webrtc', 'geoip'],
    location: ['geolocation', 'fingerprint'],
    extensions: ['companion'],
    captcha: ['captcha'],
    content: ['block_ads', 'dismiss_cookies'],
    storage: ['download_max_mb', 'retention'],
    idle: ['idle'],
  };

  let {
    settings,
    extensions = [],
    compare,
  }: {
    settings: Settings;
    extensions?: { id: string; name: string; version: string }[];
    compare?: Settings;
  } = $props();

  const buildsQuery = liveQuery(async () => (await unwrap(api.GET('/v1/builds'))).builds, { kinds: [] });
  const proxiesQuery = liveQuery(async () => (await unwrap(api.GET('/v1/proxies', { params: { query: { limit: 100 } } }))).proxies, { kinds: [] });
  const poolsQuery = liveQuery(async () => (await unwrap(api.GET('/v1/proxy-pools'))).pools, { kinds: [] });
  const libraryQuery = liveQuery(async () => (await unwrap(api.GET('/v1/extensions'))).extensions, { kinds: [] });
  const defaultsQuery = liveQuery(async () => (await unwrap(api.GET('/v1/settings'))).settings, { kinds: [] });

  const profileExtensions = $derived(
    extensions.map((ext) => ({ extension_id: ext.id, auto_update: false, pinned_version: ext.version })),
  );

  const changed = (id: string) => {
    if (!compare) return false;
    const fields = SECTION_FIELDS[id] ?? [];
    return fields.some((field) => canonical(settings[field]) !== canonical(compare[field]));
  };

  const FIELD_LABELS: Record<string, string> = {
    build: 'Browser version',
    display: 'Display',
    viewport: 'Viewport',
    screen: 'Screen',
    proxy: 'Proxy',
    locale: 'Locale',
    timezone: 'Timezone',
    args: 'Launch arguments',
    env: 'Environment',
    prefs: 'Preferences',
    user_agent: 'User agent',
    webrtc: 'WebRTC',
    geoip: 'Match location to proxy',
    geolocation: 'Geolocation',
    fingerprint: 'Fingerprint',
    companion: 'Companion extension',
    captcha: 'Captcha policy',
    block_ads: 'Block ads',
    dismiss_cookies: 'Dismiss cookie banners',
    download_max_mb: 'Download size limit',
    retention: 'Retention',
    idle: 'Idle timeouts',
  };

  const formatValue = (value: unknown): string => {
    if (value === null || value === undefined || value === '') return 'Not set';
    if (typeof value === 'object') return JSON.stringify(value);
    return String(value);
  };

  // Field-level diff for one section: snapshot value -> current ("now") value.
  const fieldDiffs = (id: string) => {
    if (!compare) return [];
    return (SECTION_FIELDS[id] ?? [])
      .filter((field) => canonical(settings[field]) !== canonical(compare[field]))
      .map((field) => ({ label: FIELD_LABELS[field] ?? field, from: formatValue(settings[field]), to: formatValue(compare[field]) }));
  };



  const sectionProps = $derived({
    settings,
    readonly: true,
    errors: {},
    builds: buildsQuery.data ?? [],
    proxies: proxiesQuery.data ?? [],
    pools: poolsQuery.data ?? [],
    library: libraryQuery.data ?? [],
    defaults: defaultsQuery.data,
    profileExtensions,
    onExtensions: () => {},
    extUnsupported: false,
    isTemplate: false,
    pinned: false,
    onPinned: () => {},
    quotaGb: null,
    onQuota: () => {},
    reportInvalid: () => {},
  } satisfies SectionProps);
</script>

{#snippet diffs(id: string)}
  {#if fieldDiffs(id).length > 0}
    <ul class="diffs" aria-label="Changed fields">
      {#each fieldDiffs(id) as diff (diff.label)}
        <li><strong>{diff.label}:</strong> {diff.from} → now {diff.to}</li>
      {/each}
    </ul>
  {/if}
{/snippet}

<div class="settings-view">
  {#each BASIC_SECTIONS as section (section.id)}
    <Card id={section.id} title={section.title} description={section.description}>
      {#snippet actions()}
        {#if changed(section.id)}<Badge tone="warning">Changed</Badge>{/if}
      {/snippet}
      <section.component {...sectionProps} />
      {@render diffs(section.id)}
    </Card>
  {/each}

  <details class="advanced" open={Boolean(compare)}>
    <summary>
      <Icon name="chevron-down" />
      <span class="advanced-title">Advanced</span>
      <span class="advanced-note">{ADVANCED_NOTE}</span>
    </summary>
    <div class="advanced-body">
      {#each ADVANCED_SECTIONS as section (section.id)}
        <Card id={section.id} title={section.title} description={section.description}>
          {#snippet actions()}
            {#if changed(section.id)}<Badge tone="warning">Changed</Badge>{/if}
          {/snippet}
          <section.component {...sectionProps} />
          {@render diffs(section.id)}
        </Card>
      {/each}
    </div>
  </details>

  {#if extensions.length > 0}
    <Card title="Extensions used">
      <ul class="used">
        {#each extensions as ext (ext.id)}
          <li>{ext.name} <span class="version">{ext.version}</span></li>
        {/each}
      </ul>
    </Card>
  {/if}
</div>

<style>
  .settings-view {
    display: flex;
    flex-direction: column;
    gap: 24px;
    min-width: 0;
  }

  .advanced summary {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    cursor: pointer;
    list-style: none;
  }

  .advanced summary::-webkit-details-marker {
    display: none;
  }

  .advanced summary :global(svg) {
    transform: rotate(-90deg);
    transition: transform var(--duration-fast) var(--ease-standard);
  }

  .advanced[open] summary :global(svg) {
    transform: none;
  }

  .advanced-title {
    font-weight: 600;
  }

  .advanced-note {
    color: var(--text-secondary);
  }

  .advanced-body {
    display: flex;
    flex-direction: column;
    gap: 24px;
    margin-top: var(--space-4);
  }

  .used {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding-left: var(--space-5);
  }

  .diffs {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: var(--space-4) 0 0;
    padding: var(--space-3) var(--space-4) var(--space-3) var(--space-6);
    background: var(--warning-bg, var(--bg-subtle));
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
  }

  .diffs strong {
    color: var(--text);
    font-weight: 500;
  }

  .version {
    color: var(--text-secondary);
  }
</style>
