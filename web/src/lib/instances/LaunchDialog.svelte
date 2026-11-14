<script lang="ts">
  import { ApiError, api, unwrap } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Icon from '$lib/ui/Icon.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Switch from '$lib/ui/Switch.svelte';
  import { lastLaunchedProfile, launchProfile } from './launch';

  type Profile = components['schemas']['Profile'];

  let { open = $bindable(false), profileId }: { open?: boolean; profileId?: string } = $props();

  const DEFAULT_PRIORITY = 5;
  const MAX_PRIORITY = 9;
  const displayOptions = [
    { value: '', label: 'Profile default' },
    { value: 'headless', label: 'Headless (no window)' },
    { value: 'visible', label: 'Visible (a window on this computer)' },
    { value: 'virtual', label: 'Virtual (off-screen display)' },
  ];
  const admissionOptions = [
    { value: 'queue', label: 'Wait in line' },
    { value: 'reject', label: 'Fail right away' },
    { value: 'evict', label: 'Make room (stop the least recently used idle browser)' },
  ];

  let profiles = $state<Profile[]>([]);
  let profile = $state('');
  let display = $state('');
  let url = $state('');
  let priority = $state<string | number | null>(DEFAULT_PRIORITY);
  let admission = $state('queue');
  let exclusive = $state(false);
  let failure = $state<string | null>(null);
  let profilesLoaded = $state(false);
  let priorityError = $state<string | null>(null);

  const selected = $derived(profileId ?? profile);
  const selectedProfile = $derived(profiles.find((p) => p.id === selected));
  // Built-in profiles (the one-off pool) are not something a person launches by hand.
  const pickable = $derived(profiles.filter((p) => !p.builtin));

  $effect(() => {
    if (!open) return;

    failure = null;
    priorityError = null;
    display = '';
    url = '';
    priority = DEFAULT_PRIORITY;
    exclusive = false;
    profile = profileId ?? '';

    void unwrap(api.GET('/v1/settings')).then(
      ({ settings }) => (admission = settings.admission?.default ?? 'queue'),
      () => (admission = 'queue'),
    );
    profilesLoaded = false;
    loadProfiles();
  });

  function loadProfiles() {
    failure = null;
    void unwrap(api.GET('/v1/profiles')).then(
      ({ profiles: list }) => {
        profiles = list.toSorted((a, b) => a.name.localeCompare(b.name));
        const last = lastLaunchedProfile();
        const choices = profiles.filter((p) => !p.builtin);
        if (!profileId) profile = (choices.find((p) => p.id === last) ?? choices[0])?.id ?? '';
        profilesLoaded = true;
      },
      (e: ApiError) => (failure = `Couldn't load your profiles. ${e.message}`),
    );
  }

  // "example.com" is what people type; the daemon wants a full URL.
  const withScheme = (value: string) => (/^[a-z][a-z0-9+.-]*:/i.test(value) ? value : `https://${value}`);

  function launch(event: SubmitEvent) {
    event.preventDefault();
    const level = Number(priority);
    priorityError =
      priority !== null && priority !== '' && Number.isInteger(level) && level >= 0 && level <= MAX_PRIORITY
        ? null
        : `Enter a whole number from 0 to ${MAX_PRIORITY}.`;
    if (priorityError || !selectedProfile) return;

    const start = url.trim();
    launchProfile(selectedProfile, {
      priority: level,
      admission: admission as 'queue' | 'reject' | 'evict',
      exclusive,
      ...(display && { display: display as 'headless' | 'visible' | 'virtual' }),
      ...(start && { url: withScheme(start) }),
    });
    open = false;
  }
</script>

<Dialog bind:open title={selectedProfile ? `Launch ${selectedProfile.name}` : 'Launch a browser'}>
  <form id="launch-form" onsubmit={launch} novalidate>
    <div class="fields">
      {#if !profileId}
        <Select
          label="Profile"
          bind:value={profile}
          options={pickable.map((p) => ({ value: p.id, label: p.name }))}
          hint={profilesLoaded && pickable.length === 0 ? 'Create a profile first.' : undefined}
        />
      {/if}
      <Input label="Start page (optional)" type="text" bind:value={url} placeholder="https://example.com" autocomplete="off" />
      <Select label="Display" bind:value={display} options={displayOptions} />
      <details class="advanced">
        <summary><span class="chevron"><Icon name="chevron-right" /></span>Advanced</summary>
        <div class="fields">
          <Input
            label="Priority"
            type="number"
            min={0}
            max={MAX_PRIORITY}
            step={1}
            bind:value={priority}
            hint="Higher launches first when brr is at capacity."
            error={priorityError}
          />
          <Select label="When at capacity" bind:value={admission} options={admissionOptions} />
          <Switch label="Exclusive" bind:checked={exclusive} hint="Agents can't open tabs in this browser while you hold it." />
        </div>
      </details>
    </div>
    {#if failure}
      <p class="failure" role="alert">
        {failure}
        {#if !profilesLoaded}<button type="button" class="link" onclick={loadProfiles}>Retry</button>{/if}
      </p>
    {/if}
    <p class="note">You'll be in control of the first tab, so you can use it right away. Release it when an agent should take over.</p>
  </form>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" type="submit" form="launch-form" disabled={!selectedProfile}>Launch</Button>
  {/snippet}
</Dialog>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .failure {
    margin-top: var(--space-4);
    padding: var(--space-3);
    background: var(--danger-bg);
    border: 1px solid var(--danger-border);
    border-radius: var(--radius-md);
    color: var(--danger-fg);
  }


  .failure .link {
    margin-left: var(--space-1);
    color: inherit;
    font: inherit;
    font-weight: 500;
    text-decoration: underline;
    background: none;
    border: 0;
    padding: 0;
    cursor: pointer;
  }

  .advanced summary {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin-bottom: var(--space-3);
    cursor: pointer;
    list-style: none;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .advanced summary::-webkit-details-marker {
    display: none;
  }

  .chevron {
    display: flex;
    transition: transform var(--duration-base) var(--ease-out);
  }

  .advanced[open] .chevron {
    transform: rotate(90deg);
  }

  .note {
    margin-top: var(--space-4);
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }
</style>
