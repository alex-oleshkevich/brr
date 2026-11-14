<script lang="ts">
  import { untrack } from 'svelte';
  import { buildLabel, INSTALLABLE_SOURCES, channelsFor, SOURCE_ENGINE, SOURCE_NAMES } from '$lib/builds/labels';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import Select from '$lib/ui/Select.svelte';
  import { channelOptionLabel, missingLabel, resolveBuild, resolvedLabel } from '../resolve';
  import type { SectionProps } from '../form';

  let { settings, readonly = false, builds }: SectionProps = $props();

  let mode = $state(untrack(() => ('channel' in settings.build ? 'channel' : 'pin')));
  let channel = $state(untrack(() => ('channel' in settings.build ? settings.build.channel : 'cft:stable')));
  let pin = $state(untrack(() => ('build_id' in settings.build ? settings.build.build_id : '')));

  const source = $derived(channel.split(':')[0]);
  const engine = $derived(
    mode === 'channel'
      ? SOURCE_ENGINE[source as keyof typeof SOURCE_ENGINE]
      : (builds.find((b) => b.id === pin)?.engine ?? 'chromium'),
  );
  const pinOptions = $derived(builds.map((b) => ({ value: b.id, label: buildLabel(b) })));
  const channelOptions = $derived(
    INSTALLABLE_SOURCES.map((s) => ({
      group: SOURCE_NAMES[s],
      options: channelsFor(s).map((c) => ({ value: `${s}:${c.value}`, label: channelOptionLabel(s, c.value, builds) })),
    })),
  );
  const resolved = $derived(resolveBuild(mode === 'channel' ? { channel } : { build_id: pin }, builds));

  $effect(() => {
    settings.build = mode === 'channel' ? { channel } : { build_id: pin };
  });
</script>

<div class="fields">
  <div class="field">
    <span class="field-label">Version</span>
    <SegmentedControl
      disabled={readonly}
      label="Version"
      bind:value={mode}
      options={[
        { value: 'channel', label: 'Follow a channel' },
        { value: 'pin', label: 'Pin a browser version' },
      ]}
    />
  </div>

  {#if mode === 'channel'}
    <Select label="Channel" bind:value={channel} options={channelOptions} disabled={readonly} />
  {:else}
    <Select label="Browser version" bind:value={pin} options={pinOptions} disabled={readonly} hint={pinOptions.length === 0 ? 'No browser versions installed yet.' : undefined} />
  {/if}

  {#if resolved.kind === 'build'}
    <p class="resolved">
      Launches <strong>{resolvedLabel(mode === 'channel' ? { channel } : { build_id: pin }, resolved)}</strong>{#if resolved.fallback}. No {SOURCE_NAMES[source as keyof typeof SOURCE_NAMES]} build is installed, so the browser detected on this computer is used{/if}.
    </p>
  {:else if resolved.kind === 'missing'}
    <p class="resolved warning" role="status">{missingLabel(resolved)}. <a href="/builds">Install one</a> before launching.</p>
  {/if}
  <p class="field-hint">Engine: {engine === 'camoufox' ? 'Camoufox (Firefox)' : 'Chromium'}</p>
</div>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .resolved {
    margin: 0;
    color: var(--text-secondary);
  }

  .resolved strong {
    color: var(--text);
    font-weight: 500;
  }

  .warning {
    color: var(--warning-fg);
  }
</style>
