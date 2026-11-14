<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import type { components } from '$lib/api/schema';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import { proxyLabel, type Proxy } from './format';

  type Profile = components['schemas']['Profile'];

  let {
    open = $bindable(false),
    proxy,
    users,
    ondeleted,
  }: { open?: boolean; proxy: Proxy | null; users: Profile[]; ondeleted: () => void } = $props();

  let busy = $state(false);

  async function removeAndDelete() {
    if (!proxy || busy) return;
    busy = true;
    try {
      for (const profile of users) {
        await unwrap(api.PATCH('/v1/profiles/{profile}', { params: { path: { profile: profile.id } }, body: { profile: profile.id, settings: { ...profile.settings, proxy: null } } }));
      }
      await unwrap(api.DELETE('/v1/proxies/{proxy_id}', { params: { path: { proxy_id: proxy.id } } }));
      toast.success('Proxy deleted');
      open = false;
    } catch (e) {
      toast.apiError(e, "Couldn't delete the proxy");
    } finally {
      busy = false;
      ondeleted();
    }
  }
</script>

<Dialog
  bind:open
  size="sm"
  title="{proxy ? proxyLabel(proxy) : 'This proxy'} is in use"
  description="{users.length === 1 ? 'This profile sends' : 'These profiles send'} its traffic through it. Deleting it switches {users.length === 1 ? 'the profile' : 'them'} to no proxy; a running browser keeps using it until it restarts."
  dismissible={!busy}
>
  <ul class="users">
    {#each users as profile (profile.id)}
      <li><a href="/profiles/{profile.id}">{profile.name}</a></li>
    {/each}
  </ul>
  {#snippet footer()}
    <Button onclick={() => (open = false)} disabled={busy}>Cancel</Button>
    <Button variant="danger" loading={busy} onclick={removeAndDelete}>
      Remove from {users.length} {users.length === 1 ? 'profile' : 'profiles'} and delete
    </Button>
  {/snippet}
</Dialog>

<style>
  .users {
    margin: 0;
    padding-left: var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
</style>
