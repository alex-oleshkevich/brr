<script lang="ts">
  import { tick } from 'svelte';
  import { goto } from '$app/navigation';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import { SOURCE_NAMES } from '$lib/builds/labels';
  import { resolveBuild } from '$lib/profiles/resolve';
  import { RECOMMENDED_TEMPLATE, templatePurpose, templateTitle, type Template } from '$lib/profiles/templates';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type BuildView = components['schemas']['BuildView'];

  const KIND_HELP = {
    persistent: 'Keeps logins, cookies and history between launches. One browser at a time.',
    ephemeral: 'Starts fresh on every launch and forgets everything afterwards. Any number of browsers at once.',
  } as const;

  let { open = $bindable(false), templateId }: { open?: boolean; templateId?: string } = $props();

  let step = $state(1);
  let templates = $state<Template[]>([]);
  let builds = $state<BuildView[]>([]);
  let chosen = $state('');
  let name = $state('');
  let kind = $state<'persistent' | 'ephemeral'>('persistent');
  let loading = $state(false);
  let loadError = $state<ApiError | null>(null);
  let creating = $state(false);
  let nameError = $state<string | null>(null);

  const ordered = $derived(
    [...templates.filter((t) => t.builtin), ...templates.filter((t) => !t.builtin)].toSorted(
      (a, b) => Number(b.name === RECOMMENDED_TEMPLATE) - Number(a.name === RECOMMENDED_TEMPLATE),
    ),
  );
  const chosenTemplate = $derived(templates.find((t) => t.id === chosen));

  async function goToStep(next: number) {
    step = next;
    await tick();
    if (next === 2) document.getElementById('new-profile-name')?.focus();
    else document.querySelector<HTMLInputElement>('input[name="template"]:checked')?.focus();
  }

  $effect(() => {
    if (!open) return;
    step = templateId ? 2 : 1;
    chosen = templateId ?? '';
    name = '';
    kind = 'persistent';
    nameError = null;
    load();
  });

  function load() {
    loading = true;
    loadError = null;
    void Promise.all([
      unwrap(api.GET('/v1/templates')).then((out) => {
        templates = out.templates;
        if (!chosen) chosen = out.templates.find((t) => t.name === RECOMMENDED_TEMPLATE)?.id ?? '';
      }),
      unwrap(api.GET('/v1/builds')).then((out) => (builds = out.builds)),
    ]).then(
      () => (loading = false),
      (e) => {
        loading = false;
        loadError = e instanceof ApiError ? e : null;
      },
    );
  }

  // Set when no build of the template's browser is installed; the card then links to Browser versions.
  function unavailable(t: Template): string | null {
    const resolved = resolveBuild(t.settings.build, builds);
    return resolved.kind === 'missing' ? `Needs ${SOURCE_NAMES[resolved.source]}, which isn't installed.` : null;
  }

  async function create(event: SubmitEvent) {
    event.preventDefault();
    if (!name.trim() || !chosen || creating) return;
    creating = true;
    nameError = null;
    try {
      const profile = await unwrap(api.POST('/v1/profiles', { body: { name: name.trim(), template: chosen, kind } }));
      toast.success('Profile created');
      open = false;
      await goto(`/profiles/${profile.id}`);
    } catch (e) {
      if (e instanceof ApiError && (e.code === 'Conflict' || (e.code === 'InvalidArgument' && e.details?.field === 'name'))) {
        nameError = e.message;
      } else {
        toast.apiError(e, "Couldn't create the profile");
      }
    } finally {
      creating = false;
    }
  }
</script>

<Dialog bind:open title="New profile" size="lg">
  <p class="step-indicator">Step {step} of 2 · {step === 1 ? 'Choose a template' : 'Name and kind'}</p>
  {#if step === 2 && chosenTemplate}<p class="step-from">From {templateTitle(chosenTemplate)}</p>{/if}

  {#if step === 1}
    <h3 class="sr-only">Choose a template</h3>
    <div class="options" role="radiogroup" aria-label="Choose a template">
      {#if loading}
        {#each { length: 4 } as _, i (i)}<Skeleton height="56px" radius="var(--radius-md)" />{/each}
      {/if}
      {#each ordered as t (t.id)}
        {@const missing = unavailable(t)}
        <div class="option" class:selected={chosen === t.id}>
          <label>
            <input type="radio" name="template" value={t.id} bind:group={chosen} />
            <span class="option-text">
              <span class="option-name">{templateTitle(t)}</span>
              {#if templatePurpose(t)}<span class="option-summary">{templatePurpose(t)}</span>{/if}
              {#if missing}<span class="option-warning">{missing}</span>{/if}
            </span>
          </label>
          {#if missing}<a class="install" href="/builds">Install</a>{/if}
        </div>
      {/each}
      {#if loadError}
        <p class="load-error" role="alert">
          Couldn't load templates. {loadError.message}
          <Button size="sm" onclick={load}>Retry</Button>
        </p>
      {:else if !loading && ordered.length === 0}
        <p class="field-hint">No templates available.</p>
      {/if}
    </div>
  {:else}
    <form id="new-profile-form" onsubmit={create}>
      <div class="fields">
        <Input id="new-profile-name" label="Name" bind:value={name} required error={nameError} />
        <div class="field">
          <span class="field-label">Kind</span>
          <SegmentedControl
            label="Kind"
            bind:value={kind}
            options={[
              { value: 'persistent', label: 'Persistent' },
              { value: 'ephemeral', label: 'Ephemeral' },
            ]}
          />
          <p class="field-hint">{KIND_HELP[kind]}</p>
        </div>
      </div>
    </form>
  {/if}

  {#snippet footer()}
    {#if step === 1}
      <Button onclick={() => (open = false)}>Cancel</Button>
      <Button variant="primary" disabled={!chosen} onclick={() => goToStep(2)}>Next</Button>
    {:else}
      <Button variant="ghost" onclick={() => goToStep(1)}>Back</Button>
      <Button onclick={() => (open = false)} disabled={creating}>Cancel</Button>
      <Button variant="primary" type="submit" form="new-profile-form" loading={creating} disabled={!name.trim()}>Create profile</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .load-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3);
    background: var(--danger-bg);
    border: 1px solid var(--danger-border);
    border-radius: var(--radius-md);
    color: var(--danger-fg);
  }

  .step-from {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    margin-bottom: var(--space-3);
  }

  .step-indicator {
    color: var(--text-tertiary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
    margin-bottom: var(--space-3);
  }

  .options {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .option {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 56px;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
  }

  .option label {
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--space-3);
    cursor: pointer;
  }

  .option-warning {
    color: var(--warning-fg);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .install {
    flex: none;
    font-weight: 500;
  }

  .option input {
    accent-color: var(--accent);
  }

  .option.selected {
    border-color: var(--accent);
    background: var(--accent-subtle);
  }

  .option-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .option-name {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-weight: 500;
  }

  .option-summary {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

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
</style>
