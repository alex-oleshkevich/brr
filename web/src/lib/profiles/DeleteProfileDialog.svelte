<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import { bytes } from '$lib/format';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Profile = components['schemas']['Profile'];

  const CREDENTIALS_PAGE = 500;

  let { open = $bindable(false), profile, ondeleted }: { open?: boolean; profile: Profile | null; ondeleted: () => void } = $props();

  let size = $state<number | null>(null);
  let backups = $state<number | null>(null);
  let logins = $state<number | null>(null);
  let busy = $state(false);

  const loaded = $derived(size !== null && backups !== null && logins !== null);
  const plural = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;

  $effect(() => {
    if (!open || !profile) return;
    const id = profile.id;
    size = backups = logins = null;
    void Promise.all([
      unwrap(api.GET('/v1/storage/usage')).then((out) => (size = out.profiles.find((u) => u.profile_id === id)?.bytes ?? 0)),
      unwrap(api.GET('/v1/profiles/{profile}/backups', { params: { path: { profile: id } } })).then((out) => (backups = out.backups.length)),
      unwrap(api.GET('/v1/credentials', { params: { query: { limit: CREDENTIALS_PAGE } } })).then(
        (out) => (logins = out.credentials.filter((c) => c.profile_ids.includes(id)).length),
      ),
    ]).catch(() => {
      // The counts only inform the warning; deleting must stay possible without them.
      size ??= 0;
      backups ??= 0;
      logins ??= 0;
    });
  });

  async function remove() {
    if (!profile || busy) return;
    busy = true;
    try {
      await unwrap(api.DELETE('/v1/profiles/{profile}', { params: { path: { profile: profile.id } } }));
      toast.success('Profile deleted');
      open = false;
      ondeleted();
    } catch (e) {
      toast.apiError(e, "Couldn't delete the profile");
    } finally {
      busy = false;
    }
  }
</script>

<Dialog bind:open title="Delete {profile?.name ?? 'profile'}?" description="This can't be undone." size="sm" dismissible={!busy}>
  {#if loaded}
    <ul class="consequences">
      <li>Deletes {bytes(size)} of browser data: cookies, logins and history.</li>
      <li>
        {#if backups}
          {plural(backups, 'backup')} {backups === 1 ? 'is' : 'are'} kept. Find {backups === 1 ? 'it' : 'them'} under <a href="/storage">Storage → Backups</a>.
        {:else}
          This profile has no backups.
        {/if}
      </li>
      {#if logins}<li>{plural(logins, 'linked login')} {logins === 1 ? 'is' : 'are'} unlinked, not deleted.</li>{/if}
    </ul>
  {:else}
    <p class="field-hint" role="status">Checking what this will remove…</p>
  {/if}
  {#snippet footer()}
    <Button autofocus onclick={() => (open = false)} disabled={busy}>Cancel</Button>
    <Button variant="danger" loading={busy} onclick={remove}>Delete profile</Button>
  {/snippet}
</Dialog>

<style>
  .consequences {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding-left: var(--space-5);
  }
</style>
