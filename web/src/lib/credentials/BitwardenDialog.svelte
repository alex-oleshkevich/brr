<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Checkbox from '$lib/ui/Checkbox.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import EmptyState from '$lib/ui/EmptyState.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Skeleton from '$lib/ui/Skeleton.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Item = components['schemas']['BitwardenItem'];
  type Profile = components['schemas']['Profile'];

  const SEARCH_DELAY_MS = 250;

  let { open = $bindable(false), profiles, onsaved }: { open?: boolean; profiles: Profile[]; onsaved?: () => void } = $props();

  let query = $state('');
  let items = $state<Item[]>([]);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let picked = $state<string | null>(null);
  let linked = $state<string[]>([]);
  let saving = $state(false);

  async function load(search: string) {
    loading = true;
    loadError = null;
    try {
      items = (await unwrap(api.GET('/v1/bitwarden/items', { params: { query: { query: search.trim() || undefined } } }))).items;
    } catch (e) {
      loadError = e instanceof Error ? e.message : "Couldn't load Bitwarden items";
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (!open) return;
    picked = null;
    linked = [];
    query = '';
  });

  $effect(() => {
    if (!open) return;
    const search = query;
    const timer = setTimeout(() => void load(search), search ? SEARCH_DELAY_MS : 0);
    return () => clearTimeout(timer);
  });

  const toggle = (id: string, checked: boolean) => (linked = checked ? [...linked, id] : linked.filter((p) => p !== id));

  async function save() {
    if (!picked || saving) return;
    saving = true;
    try {
      await unwrap(api.POST('/v1/bitwarden/link', { body: { item_id: picked, profile_ids: linked } }));
      toast.success('Login added from Bitwarden');
      open = false;
      onsaved?.();
    } catch (e) {
      toast.apiError(e, "Couldn't add the login");
    } finally {
      saving = false;
    }
  }
</script>

<Dialog bind:open title="Add from Bitwarden" size="lg">
  <div class="fields">
    <Input label="Search items" type="search" bind:value={query} placeholder="Name, site or username" autocomplete="off" />

    <fieldset class="items" aria-busy={loading}>
      <legend class="field-label">Bitwarden item</legend>
      {#if loadError}
        <EmptyState tone="danger" compact title="Couldn't load Bitwarden items" description={loadError}>
          {#snippet action()}<Button size="sm" onclick={() => load(query)}>Retry</Button>{/snippet}
        </EmptyState>
      {:else if loading && items.length === 0}
        <Skeleton height="48px" radius="var(--radius-md)" />
      {:else if items.length === 0}
        <EmptyState compact title={query ? 'No items match' : 'No login items in this vault'} />
      {:else}
        {#each items as item (item.id)}
          <label class="item" class:picked={picked === item.id}>
            <input type="radio" name="bitwarden-item" value={item.id} bind:group={picked} />
            <span class="main">
              <span class="name">{item.name}</span>
              <span class="meta">{[item.uris[0], item.username].filter(Boolean).join(' · ') || '—'}</span>
            </span>
            <span class="badges">
              {#if item.has_totp}<Badge>TOTP</Badge>{/if}
              {#if item.linked}<Badge tone="success">Already linked</Badge>{/if}
            </span>
          </label>
        {/each}
      {/if}
    </fieldset>

    <div class="field">
      <span class="field-label">Use in profiles (optional)</span>
      <div class="profiles">
        {#each profiles as profile (profile.id)}
          <Checkbox label={profile.name} checked={linked.includes(profile.id)} onchange={(checked) => toggle(profile.id, checked)} />
        {/each}
        {#if profiles.length === 0}<p class="field-hint">No profiles yet.</p>{/if}
      </div>
    </div>
  </div>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={saving}>Cancel</Button>
    <Button variant="primary" loading={saving} disabled={!picked} onclick={save}>Add login</Button>
  {/snippet}
</Dialog>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .items {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 260px;
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
    overflow: auto;
  }

  .item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    cursor: pointer;
  }

  .item:hover,
  .item.picked {
    background: var(--bg-muted);
  }

  .item:has(input:focus-visible) {
    outline: 2px solid var(--focus-ring, var(--accent));
    outline-offset: 1px;
  }

  .main {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }

  .name {
    font-weight: 500;
  }

  .meta {
    overflow: hidden;
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badges {
    display: flex;
    gap: var(--space-1);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .profiles {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: 160px;
    overflow: auto;
  }
</style>
