<script lang="ts">
  import { api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import { fetchAll } from '$lib/api/paged.svelte';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import StateBadge from '$lib/ui/StateBadge.svelte';
  import Switch from '$lib/ui/Switch.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { clock } from '$lib/clock.svelte';
  import { relative } from '$lib/format';
  import { confirm } from '$lib/ui/confirm.svelte';
  import type { components } from '$lib/api/schema';
  import LoginCheckDialog from '../LoginCheckDialog.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  let { profileId }: { profileId: string } = $props();

  const CREDENTIALS_PAGE = 500;

  let linkId = $state('');
  let busy = $state(false);

  const credentials = liveQuery(
    () =>
      fetchAll(async (cursor) => {
        const page = await unwrap(api.GET('/v1/credentials', { params: { query: { limit: CREDENTIALS_PAGE, cursor } } }));
        return { items: page.credentials, next: page.next_cursor };
      }),
    { kinds: ['login.*', 'profile.*'] },
  );
  const checks = liveQuery(
    async () =>
      profileId ? (await unwrap(api.GET('/v1/profiles/{profile}/login-checks', { params: { path: { profile: profileId } } }))).login_checks : [],
    { kinds: ['login.check_result', 'login.performed'], match: (e) => (e.data as { profile_id?: string } | null)?.profile_id === profileId },
  );

  type LoginCheck = components['schemas']['LoginCheck'];

  // The daemon stops auto re-login after 3 failures but keeps no flag on the check; the events say so.
  // A site is paused when its latest suspension is newer than its latest ok result.
  const paused = liveQuery(
    async () => {
      const out = await unwrap(
        api.GET('/v1/events', { params: { query: { kinds: 'login.relogin_suspended,login.check_result', order: 'desc', limit: 200 } } }),
      );
      const suspended = new Map<string, number>();
      const ok = new Map<string, number>();
      for (const e of out.events) {
        const data = e.data as { profile_id?: string; site?: string; status?: string } | null;
        if (!data?.site || data.profile_id !== profileId) continue;
        const book = e.kind === 'login.relogin_suspended' ? suspended : e.kind === 'login.check_result' && data.status === 'ok' ? ok : null;
        if (book && !book.has(data.site)) book.set(data.site, e.seq);
      }
      return new Set([...suspended].filter(([site, seq]) => seq > (ok.get(site) ?? 0)).map(([site]) => site));
    },
    { kinds: ['login.relogin_suspended', 'login.check_result'] },
  );
  let editing = $state<LoginCheck | undefined>(undefined);
  let dialogOpen = $state(false);

  const proof = (c: LoginCheck) => (c.expect_text ? `Text "${c.expect_text}"` : c.expect_selector ? `Element ${c.expect_selector}` : '—');
  const credentialLabel = (id: string | null | undefined) => {
    const cred = (credentials.data ?? []).find((c) => c.id === id);
    return cred ? `${cred.site} · ${cred.username}` : '—';
  };

  function add() {
    editing = undefined;
    dialogOpen = true;
  }

  function edit(check: LoginCheck) {
    editing = check;
    dialogOpen = true;
  }

  async function toggleRelogin(check: LoginCheck) {
    busy = true;
    try {
      await unwrap(
        api.PATCH('/v1/login-checks/{login_check_id}', {
          params: { path: { login_check_id: check.id } },
          body: { auto_relogin: !check.auto_relogin, credential_id: check.credential_id ?? null, expect_selector: check.expect_selector ?? null, expect_text: check.expect_text ?? null },
        }),
      );
      toast.success(check.auto_relogin ? 'Auto re-login off' : 'Auto re-login on');
    } catch (e) {
      toast.apiError(e, "Couldn't change auto re-login");
    } finally {
      // The switch flips itself first, so a failed PATCH needs the server's value back.
      checks.reload();
      busy = false;
    }
  }

  async function run(check: LoginCheck) {
    try {
      await unwrap(api.POST('/v1/login-checks/{login_check_id}/run', { params: { path: { login_check_id: check.id } }, body: {} }));
      checks.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't run the login check");
    }
  }

  async function remove(check: LoginCheck) {
    const ok = await confirm({
      title: `Delete the login check for ${check.site}?`,
      description: "This can't be undone.",
      confirmLabel: 'Delete login check',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await unwrap(api.DELETE('/v1/login-checks/{login_check_id}', { params: { path: { login_check_id: check.id } } }));
      toast.success('Login check deleted');
      checks.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't delete the login check");
    }
  }

  const linked = $derived((credentials.data ?? []).filter((c) => profileId && c.profile_ids.includes(profileId)));
  const unlinked = $derived((credentials.data ?? []).filter((c) => !(profileId && c.profile_ids.includes(profileId))));

  async function setLinks(credId: string, ids: string[], message: string) {
    if (busy) return;
    busy = true;
    try {
      await unwrap(api.PATCH('/v1/credentials/{credential_id}', { params: { path: { credential_id: credId } }, body: { profile_ids: ids } }));
      toast.success(message);
      credentials.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't update the links");
    } finally {
      busy = false;
    }
  }

  function unlink(credId: string, site: string) {
    if (!profileId) return;
    const cred = linked.find((c) => c.id === credId);
    if (!cred) return;
    void setLinks(credId, cred.profile_ids.filter((id) => id !== profileId), `Unlinked ${site}`);
  }

  function link() {
    if (!profileId || !linkId) return;
    const cred = unlinked.find((c) => c.id === linkId);
    if (!cred) return;
    void setLinks(cred.id, [...cred.profile_ids, profileId], `Linked ${cred.site}`);
    linkId = '';
  }
</script>

<LoginCheckDialog bind:open={dialogOpen} {profileId} check={editing} credentials={linked} onsaved={() => checks.reload()} />
<div class="stack">
  <Card title="Linked logins" description="Site logins agents can use in this profile without ever seeing the password. Changes here apply immediately.">
    <div class="fields">
      {#if linked.length === 0}
        <p class="field-hint">None linked yet.</p>
      {:else}
        <ul class="linked">
          {#each linked as cred (cred.id)}
            <li>
              <span class="truncate">{cred.site} · {cred.username}</span>
              <Button variant="ghost" size="sm" disabled={busy} onclick={() => unlink(cred.id, cred.site)}>Unlink</Button>
            </li>
          {/each}
        </ul>
      {/if}
      <div class="add">
        <Select
          label="Link a login"
          bind:value={linkId}
          options={[{ value: '', label: 'Choose a login' }, ...unlinked.map((c) => ({ value: c.id, label: `${c.site} · ${c.username}` }))]}
        />
        <Button size="sm" disabled={busy || !linkId} onclick={link}>Link</Button>
      </div>
      <p class="field-hint">Add new logins on the <a href="/credentials">Logins</a> page.</p>
    </div>
  </Card>

  <Card title="Login checks" description="A login check proves this profile is still signed in. Changes here apply immediately.">
    <div class="fields">
      <Table
        caption="Login checks"
        columns={[
          { key: 'site', label: 'Site', width: '15%' },
          { key: 'url', label: 'URL', width: '16%' },
          { key: 'proof', label: 'Proves login by', width: '16%' },
          { key: 'credential', label: 'Login', width: '14%' },
          { key: 'auto', label: 'Auto re-login', width: '11%' },
          { key: 'status', label: 'Status', width: '20%' },
          { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
        ]}
        rows={checks.data}
        key={(row: LoginCheck) => row.id}
        loading={checks.loading}
        error={checks.error}
        onretry={() => checks.reload()}
        empty={{ title: 'No login checks yet', description: 'A login check proves this profile is still signed in.' }}
      >
        {#snippet row(check: LoginCheck)}
          <td class="truncate">{check.site}</td>
          <td class="truncate" title={check.url}>{check.url}</td>
          <td class="truncate">{proof(check)}</td>
          <td class="truncate">{credentialLabel(check.credential_id)}</td>
          <td>
            <Switch label="Auto re-login for {check.site}" hideLabel checked={check.auto_relogin} disabled={busy} onchange={() => toggleRelogin(check)} />
          </td>
          <td>
            <span class="status">
              <StateBadge kind="login" state={check.last_status} />
              {#if check.last_checked_at}<span class="when">{relative(check.last_checked_at, clock.now)}</span>{/if}
            </span>
            {#if check.auto_relogin && paused.data?.has(check.site)}
              <span class="paused">Re-login paused after 3 failures</span>
            {/if}
          </td>
          <td>
            <Menu
              label="Actions for the login check on {check.site}"
              items={[
                { label: 'Run now', onselect: () => run(check) },
                { label: 'Edit…', onselect: () => edit(check) },
                { label: 'Delete…', tone: 'danger', onselect: () => remove(check) },
              ]}
            />
          </td>
        {/snippet}
      </Table>
      <div><Button size="sm" onclick={add}>Add login check</Button></div>
    </div>
  </Card>
</div>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: 24px;
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .linked {
    list-style: none;
    margin: 0;
    padding: 0;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .linked li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-height: 32px;
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    white-space: nowrap;
  }

  .paused {
    display: block;
    margin-top: var(--space-1);
    color: var(--warning-fg);
    font-size: var(--text-sm-size);
    line-height: var(--text-sm-line);
  }

  .when {
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
  }

  .add {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--space-2);
    align-items: end;
  }
</style>
