<script lang="ts">
  import PagedFooter from '$lib/ui/PagedFooter.svelte';
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { pagedQuery } from '$lib/api/paged.svelte';
  import { clock } from '$lib/clock.svelte';
  import BitwardenDialog from '$lib/credentials/BitwardenDialog.svelte';
  import CredentialDialog from '$lib/credentials/CredentialDialog.svelte';
  import ImportCsvDialog from '$lib/credentials/ImportCsvDialog.svelte';
  import LinkDialog from '$lib/credentials/LinkDialog.svelte';
  import { relative } from '$lib/format';
  import Badge from '$lib/ui/Badge.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import PageHeader from '$lib/ui/PageHeader.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import type { components } from '$lib/api/schema';

  type Credential = components['schemas']['Credential'];
  type Profile = components['schemas']['Profile'];

  const LIMIT = 100;

  let dialogOpen = $state(false);
  let editing = $state<Credential | null>(null);
  let importOpen = $state(false);
  let bitwardenOpen = $state(false);
  let linkOpen = $state(false);
  let linking = $state<Credential | null>(null);

  const list = pagedQuery(
    async (cursor) => {
      const page = await unwrap(api.GET('/v1/credentials', { params: { query: { limit: LIMIT, cursor } } }));
      return { items: page.credentials, next: page.next_cursor };
    },
    { kinds: ['login.*', 'profile.*'], key: (cred) => cred.id, moreError: "Couldn't load more logins" },
  );
  const profilesQuery = liveQuery(async () => (await unwrap(api.GET('/v1/profiles'))).profiles, { kinds: ['profile.*'] });

  const bitwarden = liveQuery(() => unwrap(api.GET('/v1/bitwarden/status')), { kinds: ['bitwarden.synced', 'bitwarden.sync_failed'] });

  const profiles = $derived(profilesQuery.data ?? []);
  const linkable = $derived(profiles.filter((p) => !p.builtin));
  const profileName = (id: string) => profiles.find((p) => p.id === id)?.name ?? id;

  function add() {
    editing = null;
    dialogOpen = true;
  }

  function edit(cred: Credential) {
    editing = cred;
    dialogOpen = true;
  }

  function link(cred: Credential) {
    linking = cred;
    linkOpen = true;
  }

  // Login checks of the linked Profiles that use this login; they stop working once it is gone.
  async function affectedChecks(cred: Credential): Promise<string[] | null> {
    try {
      const perProfile = await Promise.all(
        cred.profile_ids.map(async (id) => {
          const out = await unwrap(api.GET('/v1/profiles/{profile}/login-checks', { params: { path: { profile: id } } }));
          return out.login_checks.filter((c) => c.credential_id === cred.id).map((c) => `${profileName(id)}: ${c.url}`);
        }),
      );
      return perProfile.flat();
    } catch {
      return null;
    }
  }

  async function remove(cred: Credential) {
    const names = cred.profile_ids.map(profileName);
    const checks = await affectedChecks(cred);
    const linked = names.length === 0 ? "It isn't linked to any Profile." : `It is linked to ${names.join(', ')}.`;
    const effect =
      checks === null
        ? ' Login checks that use it stop working.'
        : checks.length === 0
          ? ' No login checks use it.'
          : ` These login checks stop working: ${checks.join('; ')}.`;
    const ok = await confirm({
      title: `Delete the login for ${cred.site}?`,
      description: `${linked}${effect} This can't be undone.`,
      confirmLabel: 'Delete login',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await unwrap(api.DELETE('/v1/credentials/{credential_id}', { params: { path: { credential_id: cred.id } } }));
      toast.success('Login deleted');
      list.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't delete the login");
    }
  }
</script>

<PageHeader title="Logins" description="A stored username, password and optional one-time code that brr types into a site for a Profile. Secrets are never shown here or to agents.">
  {#snippet actions()}
    {#if bitwarden.data?.connected}
      <Button onclick={() => (bitwardenOpen = true)}>Add from Bitwarden</Button>
    {/if}
    <Button onclick={() => (importOpen = true)}>Import CSV</Button>
    <Button variant="primary" icon="plus" onclick={add}>Add login</Button>
  {/snippet}
</PageHeader>

<CredentialDialog bind:open={dialogOpen} credential={editing} profiles={linkable} onsaved={() => list.reload()} />
<ImportCsvDialog bind:open={importOpen} onsaved={() => list.reload()} />
<BitwardenDialog bind:open={bitwardenOpen} profiles={linkable} onsaved={() => list.reload()} />
<LinkDialog bind:open={linkOpen} credential={linking} profiles={linkable} onsaved={() => list.reload()} />

{#snippet more()}
  <PagedFooter {list} />
{/snippet}

<Table
  caption="Logins"
  columns={[
    { key: 'site', label: 'Site', width: '26%' },
    { key: 'username', label: 'Username', width: '20%' },
    { key: 'profiles', label: 'Linked Profiles', width: '26%' },
    { key: 'login', label: 'Last check', width: '20%' },
    { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
  ]}
  rows={list.items}
  key={(row: Credential) => row.id}
  loading={list.loading}
  error={list.error}
  onretry={() => list.reload()}
  footer={list.hasMore || list.capped ? more : undefined}
  empty={{ title: 'No logins yet', description: 'Add a login or import a password export from Chrome or Firefox.' }}
>
  {#snippet row(cred: Credential)}
    <td class="truncate" title={cred.site}>
      {cred.site}
      {#if cred.source === 'bitwarden'}<Badge>Bitwarden</Badge>{/if}
    </td>
    <td class="truncate">{cred.username}</td>
    <td>
      {#if cred.profile_ids.length === 0}
        <span class="not-linked">Not linked</span>
      {:else}
        <span class="badges">
          {#each cred.profile_ids.slice(0, 2) as id (id)}<Badge>{profileName(id)}</Badge>{/each}
          {#if cred.profile_ids.length > 2}<span class="more" title={cred.profile_ids.slice(2).map(profileName).join(', ')}>+{cred.profile_ids.length - 2}</span>{/if}
        </span>
      {/if}
    </td>
    <td>
      {#if cred.last_login}
        <span class="login">
          <StateBadge kind="login" state={cred.last_login.ok ? 'ok' : 'failed'} />
          <span>{relative(cred.last_login.at, clock.now)}</span>
        </span>
      {:else}
        —
      {/if}
    </td>
    <td>
      <Menu
        label="Actions for {cred.site}"
        items={[
          { label: 'Edit…', onselect: () => edit(cred) },
          { label: 'Link to profiles…', onselect: () => link(cred) },
          { label: 'Delete…', tone: 'danger', onselect: () => remove(cred) },
        ]}
      />
    </td>
  {/snippet}
</Table>

<style>
  .not-linked {
    color: var(--text-tertiary);
  }

  .badges {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-wrap: wrap;
  }

  .more {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
  }

  .login {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }
</style>
