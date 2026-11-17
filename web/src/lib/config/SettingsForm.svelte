<script lang="ts">
  import type { Snippet } from 'svelte';
  import Card from '$lib/ui/Card.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import type { SectionProps, Settings } from './form';
  import { ADVANCED_NOTE, ADVANCED_SECTIONS, BASIC_SECTIONS } from './sections';

  let {
    settings = $bindable(),
    readonly = false,
    errors = {},
    builds = [],
    proxies = [],
    pools = [],
    library = [],
    defaults,
    profileExtensions = [],
    onExtensions = () => {},
    extUnsupported = false,
    isTemplate = false,
    pinned = $bindable(false),
    quotaGb = $bindable(null),
    profileId = null,
    profileKind = null,
    oninvalidchange,
    general,
  }: {
    settings: Settings;
    readonly?: boolean;
    errors?: Record<string, string>;
    builds?: SectionProps['builds'];
    proxies?: SectionProps['proxies'];
    pools?: SectionProps['pools'];
    library?: SectionProps['library'];
    defaults?: SectionProps['defaults'];
    profileExtensions?: SectionProps['profileExtensions'];
    onExtensions?: SectionProps['onExtensions'];
    extUnsupported?: boolean;
    isTemplate?: boolean;
    pinned?: boolean;
    quotaGb?: number | null;
    profileId?: string | null;
    profileKind?: string | null;
    oninvalidchange?: (invalid: boolean) => void;
    general?: Snippet;
  } = $props();

  const ADVANCED_IDS = new Set(['advanced', ...ADVANCED_SECTIONS.map((s) => s.id)]);

  let active = $state('general');
  let advancedOpen = $state(false);
  const invalidIds = new Set<string>();

  function reportInvalid(id: string, invalid: boolean) {
    if (invalid) invalidIds.add(id);
    else invalidIds.delete(id);
    // A problem inside the collapsed group would otherwise block Save with nothing in sight.
    if (invalid) advancedOpen = true;
    oninvalidchange?.(invalidIds.size > 0);
  }

  // A server-side rejection or a link such as #extensions can point inside the collapsed group.
  $effect(() => {
    if (Object.keys(errors).length > 0 || ADVANCED_IDS.has(location.hash.slice(1))) advancedOpen = true;
  });

  const sectionProps = $derived({
    settings,
    readonly,
    errors,
    builds,
    proxies,
    pools,
    library,
    defaults,
    profileExtensions,
    onExtensions,
    extUnsupported,
    isTemplate,
    pinned,
    onPinned: (value: boolean) => (pinned = value),
    quotaGb,
    onQuota: (value: number | null) => (quotaGb = value),
    profileId,
    profileKind,
    reportInvalid,
  } satisfies SectionProps);

  $effect(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) if (entry.isIntersecting) active = entry.target.id;
      },
      { rootMargin: '-80px 0px -60% 0px' },
    );
    for (const el of document.querySelectorAll('.settings-form .card[id]')) observer.observe(el);
    return () => observer.disconnect();
  });

  function jump(event: MouseEvent, id: string) {
    event.preventDefault();
    if (id === 'advanced') advancedOpen = true;
    document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    document.getElementById(`${id}-title`)?.focus({ preventScroll: true });
    active = id;
  }
</script>

<div class="settings-form">
  <nav class="section-nav" aria-label="Settings sections">
    <a href="#general" aria-current={active === 'general' ? 'true' : undefined} onclick={(e) => jump(e, 'general')}>Basics</a>
    {#each BASIC_SECTIONS as section (section.id)}
      <a href="#{section.id}" aria-current={active === section.id ? 'true' : undefined} onclick={(e) => jump(e, section.id)}>{isTemplate && section.templateTitle ? section.templateTitle : section.title}</a>
    {/each}
    <a href="#advanced" aria-current={ADVANCED_IDS.has(active) ? 'true' : undefined} onclick={(e) => jump(e, 'advanced')}>Advanced</a>
  </nav>

  <div class="content">
    {#if general}
      <Card id="general" title="Basics">{@render general()}</Card>
    {/if}
    {#each BASIC_SECTIONS as section (section.id)}
      <Card id={section.id} title={isTemplate && section.templateTitle ? section.templateTitle : section.title} description={section.description}>
        <section.component {...sectionProps} />
      </Card>
    {/each}

    <details id="advanced" class="advanced" bind:open={advancedOpen}>
      <summary>
        <Icon name="chevron-down" />
        <span class="advanced-title">Advanced</span>
        <span class="advanced-note">{ADVANCED_NOTE}</span>
      </summary>
      <div class="advanced-body">
        {#each ADVANCED_SECTIONS as section (section.id)}
          <Card id={section.id} title={section.title} description={section.description}>
            <section.component {...sectionProps} />
          </Card>
        {/each}
      </div>
    </details>
  </div>
</div>

<style>
  .settings-form {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr);
    gap: var(--space-6);
    align-items: start;
  }

  .section-nav {
    position: sticky;
    top: calc(var(--topbar-h) + var(--space-6));
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .section-nav a {
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-md);
    color: var(--text-secondary);
  }

  .section-nav a:hover {
    background: var(--bg-muted);
    color: var(--text);
  }

  .section-nav a[aria-current='true'] {
    background: var(--bg-muted);
    color: var(--text);
    font-weight: 500;
  }

  .content {
    display: flex;
    flex-direction: column;
    gap: 24px;
    min-width: 0;
    padding-bottom: 40vh;
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

  :global(.settings-form .card) {
    scroll-margin-top: calc(var(--topbar-h) + var(--space-6));
  }

  @media (max-width: 1023px) {
    .settings-form {
      grid-template-columns: minmax(0, 1fr);
    }

    .section-nav {
      display: none;
    }
  }
</style>
