<script lang="ts">
  import { goto } from '$app/navigation';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import NewProfileDialog from '$lib/profiles/NewProfileDialog.svelte';
  import { engineOf, type Settings } from '$lib/config/form';
  import { proxyName } from '$lib/config/proxy';
  import { NAME_HINT, templatePurpose, templateSlug, templateTitle, uniqueCopyName, type Template } from '$lib/config/templates';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import { profileNameError } from '$lib/profiles/restore';

  let creatingProfile = $state(false);
  let createTemplateId = $state<string | undefined>(undefined);
  let duplicating = $state<Template | null>(null);
  let duplicateName = $state('');
  let duplicateOpen = $state(false);
  let duplicateBusy = $state(false);
  let duplicateError = $state<string | null>(null);
  let newOpen = $state(false);
  let newName = $state('');
  let newBase = $state('');
  let newBusy = $state(false);
  let newError = $state<string | null>(null);

  const duplicateNameError = $derived(duplicateName.trim() === '' ? null : profileNameError(duplicateName.trim()));
  const newNameError = $derived(newName.trim() === '' ? null : profileNameError(newName.trim()));

  const list = liveQuery(async () => (await unwrap(api.GET('/v1/templates'))).templates, { kinds: [] });
  $effect(() => {
    const refresh = () => list.reload();
    window.addEventListener('focus', refresh);
    return () => window.removeEventListener('focus', refresh);
  });
  const buildsQuery = liveQuery(async () => (await unwrap(api.GET('/v1/builds'))).builds, { kinds: ['build.*'] });
  const profilesQuery = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, { kinds: ['profile.*'] });
  const proxiesQuery = liveQuery(async () => (await unwrap(api.GET('/v1/proxies', { params: { query: { limit: 100 } } }))).proxies, { kinds: [] });
  const poolsQuery = liveQuery(async () => (await unwrap(api.GET('/v1/proxy-pools'))).pools, { kinds: [] });
  const settingsQuery = liveQuery(async () => (await unwrap(api.GET('/v1/settings'))).settings, { kinds: [] });

  const builtins = $derived((list.data ?? []).filter((t) => t.builtin));
  const yours = $derived((list.data ?? []).filter((t) => !t.builtin));
  const profileCount = $derived.by(() => {
    const counts = new Map<string, number>();
    const oneshotName = settingsQuery.data?.oneshot.profile ?? 'oneshot';
    for (const p of profilesQuery.data ?? []) {
      if (p.builtin || p.name === oneshotName) continue;
      if (p.template_id) counts.set(p.template_id, (counts.get(p.template_id) ?? 0) + 1);
    }
    return counts;
  });

  function proxyLabel(settings: Settings): string {
    const choice = settings.proxy;
    if (!choice) return 'No proxy';
    if ('proxy_id' in choice) {
      const proxy = (proxiesQuery.data ?? []).find((p) => p.id === choice.proxy_id);
      return proxy ? proxyName(proxy) : 'Proxy';
    }
    if ('pool_id' in choice) {
      const pool = (poolsQuery.data ?? []).find((p) => p.id === choice.pool_id);
      return pool ? `Pool: ${pool.name}` : 'Proxy pool';
    }
    return 'Proxy';
  }

  function newFromTemplate(t: Template) {
    createTemplateId = t.id;
    creatingProfile = true;
  }

  function openDuplicate(t: Template) {
    duplicating = t;
    const base = t.builtin ? templateSlug(templateTitle(t)) : t.name;
    duplicateName = uniqueCopyName(base, new Set((list.data ?? []).map((x) => x.name)));
    duplicateError = null;
    duplicateOpen = true;
  }

  async function duplicate(event: SubmitEvent) {
    event.preventDefault();
    if (!duplicating || !duplicateName.trim() || duplicateBusy) return;
    duplicateBusy = true;
    duplicateError = null;
    try {
      const created = await unwrap(
        api.POST('/v1/templates', { body: { name: duplicateName.trim(), settings: duplicating.settings, extensions: duplicating.extensions ?? [], description: duplicating.description } }),
      );
      toast.success('Template duplicated');
      duplicateOpen = false;
      list.reload();
      await goto(`/templates/${created.id}`);
    } catch (e) {
      if (e instanceof ApiError && (e.code === 'Conflict' || (e.code === 'InvalidArgument' && e.details?.field === 'name'))) {
        duplicateError = e.message;
      } else {
        toast.apiError(e, "Couldn't duplicate the template");
      }
    } finally {
      duplicateBusy = false;
    }
  }

  async function remove(t: Template) {
    const ok = await confirm({
      title: `Delete ${templateTitle(t)}?`,
      description: 'Profiles made from it keep their settings. New profiles can no longer be made from it.',
      confirmLabel: 'Delete template',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await unwrap(api.DELETE('/v1/templates/{template}', { params: { path: { template: t.id } } }));
      toast.success('Template deleted');
      list.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't delete the template");
    }
  }

  function openNew() {
    newName = '';
    newError = null;
    newBase = builtins[0]?.id ?? '';
    newOpen = true;
  }

  let lastSuggestion = $state('');
  $effect(() => {
    if (!newOpen) return;
    const base = builtins.find((t) => t.id === newBase);
    const suggestion =
      newBase === 'blank-chromium' ? 'chrome-defaults' : newBase === 'blank-camoufox' ? 'camoufox-defaults' : base ? templateSlug(templateTitle(base)) : '';
    if (suggestion && (newName === '' || newName === lastSuggestion)) newName = suggestion;
    lastSuggestion = suggestion;
  });

  async function createTemplate(event: SubmitEvent) {
    event.preventDefault();
    if (!newName.trim() || newBusy) return;
    const blankEngine = newBase === 'blank-chromium' ? 'chromium' : newBase === 'blank-camoufox' ? 'camoufox' : null;
    const base = blankEngine
      ? builtins.find((t) => engineOf(t.settings, buildsQuery.data ?? []) === blankEngine)
      : builtins.find((t) => t.id === newBase) ?? yours.find((t) => t.id === newBase);
    if (!base) {
      newError = 'Choose a starting point.';
      return;
    }
    newBusy = true;
    newError = null;
    try {
      const created = await unwrap(
        api.POST('/v1/templates', {
          body: { name: newName.trim(), settings: base.settings, extensions: blankEngine ? [] : (base.extensions ?? []) },
        }),
      );
      toast.success('Template created');
      newOpen = false;
      list.reload();
      await goto(`/templates/${created.id}`);
    } catch (e) {
      if (e instanceof ApiError && (e.code === 'Conflict' || (e.code === 'InvalidArgument' && e.details?.field === 'name'))) {
        newError = e.message;
      } else {
        toast.apiError(e, "Couldn't create the template");
      }
    } finally {
      newBusy = false;
    }
  }
</script>

<PageHeader title="Templates" description="Starting points for new profiles. A profile copies its template once and does not follow later changes.">
  {#snippet actions()}
    <Button variant="primary" icon="plus" onclick={openNew}>New template</Button>
  {/snippet}
</PageHeader>

<NewProfileDialog bind:open={creatingProfile} templateId={createTemplateId} />

<Dialog bind:open={duplicateOpen} title="Duplicate template">
  <form id="duplicate-template-form" onsubmit={duplicate}>
    <Input label="Name" bind:value={duplicateName} required error={duplicateError ?? duplicateNameError} hint={NAME_HINT} />
  </form>
  {#snippet footer()}
    <Button onclick={() => (duplicateOpen = false)} disabled={duplicateBusy}>Cancel</Button>
    <Button variant="primary" type="submit" form="duplicate-template-form" loading={duplicateBusy} disabled={!duplicateName.trim() || duplicateNameError !== null}>Duplicate</Button>
  {/snippet}
</Dialog>

<Dialog bind:open={newOpen} title="New template" size="lg">
  <form id="new-template-form" onsubmit={createTemplate}>
    <div class="fields">
      <div class="field">
        <span class="field-label" id="new-template-base-label">Start from</span>
        <div class="options" role="radiogroup" aria-labelledby="new-template-base-label">
          {#each builtins as t (t.id)}
            <label class="option" class:selected={newBase === t.id}>
              <input type="radio" name="base" value={t.id} bind:group={newBase} />
              <span>
                <span class="option-name">{templateTitle(t)}</span>
                {#if templatePurpose(t)}<span class="option-summary">{templatePurpose(t)}</span>{/if}
              </span>
            </label>
          {/each}
          <label class="option" class:selected={newBase === 'blank-chromium'}>
            <input type="radio" name="base" value="blank-chromium" bind:group={newBase} />
            <span class="option-name">Start from Chrome defaults</span>
          </label>
          <label class="option" class:selected={newBase === 'blank-camoufox'}>
            <input type="radio" name="base" value="blank-camoufox" bind:group={newBase} />
            <span class="option-name">Start from Camoufox defaults</span>
          </label>
        </div>
      </div>
      <Input label="Name" bind:value={newName} required error={newError ?? newNameError} hint={NAME_HINT} />
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={() => (newOpen = false)} disabled={newBusy}>Cancel</Button>
    <Button variant="primary" type="submit" form="new-template-form" loading={newBusy} disabled={!newName.trim() || !newBase || newNameError !== null}>Create template</Button>
  {/snippet}
</Dialog>

{#if list.loading && !list.data}
  <div class="grid" aria-busy="true">
    {#each { length: 4 } as _, i (i)}<Skeleton height="150px" radius="var(--radius-lg)" />{/each}
  </div>
{:else if list.error && !list.data}
  <EmptyState tone="danger" title="Couldn't load templates" description={list.error.message}>
    {#snippet action()}<Button onclick={() => list.reload()}>Retry</Button>{/snippet}
  </EmptyState>
{:else}
  {#snippet card(t: Template)}
    <article class="card">
      <div class="card-head">
        <h3><a href="/templates/{t.id}">{templateTitle(t)}</a></h3>
        <Menu
          label="Actions for {templateTitle(t)}"
          items={[
            t.builtin
              ? { label: 'Duplicate', onselect: () => openDuplicate(t) }
              : { label: 'Edit', onselect: () => goto(`/templates/${t.id}`) },
            ...(t.builtin ? [] : [{ label: 'Duplicate…', onselect: () => openDuplicate(t) }]),
            { label: 'New profile from this', onselect: () => newFromTemplate(t) },
            ...(!t.builtin ? ['separator' as const, { label: 'Delete…', tone: 'danger' as const, onselect: () => remove(t) }] : []),
          ]}
        />
      </div>
      {#if templatePurpose(t)}<p class="purpose">{templatePurpose(t)}</p>{/if}
      <dl class="facts">
        <div><dt>Engine</dt><dd>{engineOf(t.settings, buildsQuery.data ?? []) === 'camoufox' ? 'Camoufox' : 'Chromium'}</dd></div>
        <div><dt>Display</dt><dd>{t.settings.display[0].toUpperCase() + t.settings.display.slice(1)}</dd></div>
        <div><dt>Proxy</dt><dd class="truncate">{proxyLabel(t.settings)}</dd></div>
        <div><dt>Extensions</dt><dd>{(t.extensions ?? []).length === 0 ? 'None' : `${(t.extensions ?? []).length}`}</dd></div>
        <div><dt>Profiles made from it</dt><dd>{profileCount.get(t.id) ?? 0}</dd></div>
      </dl>
      {#if t.builtin}<Badge>Built-in</Badge>{/if}
    </article>
  {/snippet}

  <section aria-labelledby="builtin-templates">
    <h2 id="builtin-templates">Built-in</h2>
    {#if builtins.length === 0}
      <EmptyState title="No built-in templates" description="Built-in templates appear here once brr has loaded them." />
    {:else}
      <div class="grid">{#each builtins as t (t.id)}{@render card(t)}{/each}</div>
    {/if}
  </section>

  <section aria-labelledby="your-templates">
    <h2 id="your-templates">Yours</h2>
    {#if yours.length === 0}
      <EmptyState icon="template" title="No templates of yours yet" description="Create one from a built-in starting point, then reuse it for new profiles.">
        {#snippet action()}<Button variant="primary" onclick={openNew}>New template</Button>{/snippet}
      </EmptyState>
    {:else}
      <div class="grid">{#each yours as t (t.id)}{@render card(t)}{/each}</div>
    {/if}
  </section>
{/if}

<style>
  section {
    margin-bottom: var(--space-6);
  }

  h2 {
    margin-bottom: var(--space-3);
    font-size: var(--text-md-size);
    line-height: var(--text-md-line);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: var(--space-4);
  }

  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }

  .card-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .card-head h3 {
    font-size: var(--text-base-size);
    line-height: var(--text-base-line);
  }

  .purpose {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .facts {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
  }

  .facts div {
    display: grid;
    grid-template-columns: 150px minmax(0, 1fr);
    gap: var(--space-3);
  }

  .facts dt {
    color: var(--text-secondary);
  }

  .facts dd {
    margin: 0;
    min-width: 0;
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
    min-height: 48px;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    cursor: pointer;
  }

  .option.selected {
    border-color: var(--accent);
    background: var(--accent-subtle);
  }

  .option input {
    accent-color: var(--accent);
  }

  .option-name {
    display: block;
    font-weight: 500;
  }

  .option-summary {
    display: block;
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }
</style>
