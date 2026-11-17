<script lang="ts">
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components } from '$lib/api/schema';
  import { clock } from '$lib/clock.svelte';
  import { bytes, relative } from '$lib/format';
  import { settingsHref } from '$lib/settings/sections';
  import type { Tone } from '$lib/state';
  import Button from '$lib/ui/Button.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import StatusDot from '$lib/ui/StatusDot.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';

  type Host = components['schemas']['Host'];

  const MB = 1024 * 1024;
  const MAX_BROWSERS = 64;
  const DEFAULT_BROWSERS = 4;
  const SSH_URL = /^ssh:\/\/[A-Za-z0-9._][A-Za-z0-9._-]*@[A-Za-z0-9._][A-Za-z0-9._-]*(:\d{1,5})?$/;
  const FIELDS = ['name', 'docker_url', 'max_instances'];

  // No form of its own: every change saves through its dialog.
  let { dirty = $bindable(false) }: { dirty?: boolean } = $props();

  const hostList = liveQuery(async () => (await unwrap(api.GET('/v1/hosts'))).hosts, { kinds: ['instance.state_changed'] });

  let hosts = $state<Host[]>([]);
  let checking = $state(new Set<string>());

  $effect(() => {
    hosts = hostList.data ?? [];
  });

  const mark = (id: string, on: boolean) => {
    const next = new Set(checking);
    if (on) next.add(id);
    else next.delete(id);
    checking = next;
  };

  const replace = (updated: Host) => {
    hosts = hosts.map((h) => (h.id === updated.id ? updated : h));
  };

  function status(host: Host): { tone: Tone; label: string } {
    if (checking.has(host.id)) return { tone: 'neutral', label: 'Checking…' };
    if (!host.enabled) return { tone: 'neutral', label: 'Disabled' };
    if (!host.status.checked_at) return { tone: 'neutral', label: 'Not checked' };
    if (!host.status.reachable) return { tone: 'danger', label: 'Unreachable' };
    if (host.status.image_present === false) return { tone: 'warning', label: 'Image missing' };
    if (host.status.error) return { tone: 'warning', label: 'Reachable' };
    return { tone: 'success', label: 'Ready' };
  }

  const statusDetail = (host: Host) =>
    [host.status.error, host.status.checked_at ? `checked ${relative(host.status.checked_at, clock.now)}` : null]
      .filter(Boolean)
      .map((part) => ` · ${part}`)
      .join('');

  function memory(host: Host) {
    const { mem_total_mb: total, mem_available_mb: available } = host.status;
    if (total == null || available == null) return '—';
    return `${bytes((total - available) * MB)} / ${bytes(total * MB)}`;
  }

  const image = (host: Host) => (host.status.image_present == null ? '—' : host.status.image_present ? 'Present' : 'Missing');

  async function check(host: Host) {
    mark(host.id, true);
    try {
      replace(await unwrap(api.POST('/v1/hosts/{host}/check', { params: { path: { host: host.id } }, body: {} })));
    } catch (e) {
      toast.apiError(e, "Couldn't check the host");
    } finally {
      mark(host.id, false);
    }
  }

  async function remove(host: Host) {
    const ok = await confirm({
      title: `Delete host ${host.name}?`,
      description: "brr stops placing browsers on it. Nothing on the machine is changed. This can't be undone.",
      confirmLabel: 'Delete host',
      tone: 'danger',
    });
    if (!ok) return;

    try {
      await unwrap(api.DELETE('/v1/hosts/{host}', { params: { path: { host: host.id } } }));
      toast.success('Host deleted');
      hostList.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't delete the host");
    }
  }

  let dialogOpen = $state(false);
  let editing = $state<Host | undefined>();
  let name = $state('');
  let dockerUrl = $state('');
  let maxInstances = $state<number | null>(DEFAULT_BROWSERS);
  let errors = $state<Record<string, string>>({});
  let saving = $state(false);

  function openDialog(host?: Host) {
    editing = host;
    name = host?.name ?? '';
    dockerUrl = host?.docker_url ?? '';
    maxInstances = host?.max_instances ?? DEFAULT_BROWSERS;
    errors = {};
    dialogOpen = true;
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    errors = {};
    if (!name.trim()) errors.name = 'Enter a name.';
    if (!SSH_URL.test(dockerUrl.trim())) errors.docker_url = 'Enter ssh://user@host or ssh://user@host:port.';
    if (maxInstances === null || !Number.isInteger(maxInstances) || maxInstances < 1 || maxInstances > MAX_BROWSERS)
      errors.max_instances = `Enter a whole number from 1 to ${MAX_BROWSERS}.`;
    if (Object.keys(errors).length) return;

    saving = true;
    const body = { name: name.trim(), docker_url: dockerUrl.trim(), max_instances: maxInstances as number };
    try {
      const saved = editing
        ? await unwrap(api.PATCH('/v1/hosts/{host}', { params: { path: { host: editing.id } }, body }))
        : await unwrap(api.POST('/v1/hosts', { body }));
      toast.success(editing ? 'Host saved' : 'Host added');
      dialogOpen = false;
      if (editing) replace(saved);
      else hostList.reload();
    } catch (e) {
      const field = e instanceof ApiError && e.code === 'InvalidArgument' ? (e.details as { field?: string }).field : undefined;
      if (field && FIELDS.includes(field)) errors = { [field]: (e as ApiError).message };
      else if (e instanceof ApiError && e.code === 'Conflict') errors = { name: 'A host with this name exists.' };
      else toast.apiError(e, "Couldn't save the host");
    } finally {
      saving = false;
    }
  }

  const columns = [
    { key: 'name', label: 'Name', width: '14%' },
    { key: 'url', label: 'SSH URL', width: '22%' },
    { key: 'status', label: 'Status' },
    { key: 'memory', label: 'Memory used', width: '15%', align: 'right' as const },
    { key: 'browsers', label: 'Browsers', width: '10%', align: 'right' as const },
    { key: 'image', label: 'Image', width: '9%' },
    { key: 'actions', label: 'Actions', width: '150px', srOnly: true, keepWidth: true },
  ];
</script>

<div class="stack">
  <div class="head">
    <p class="note">Hosts run browsers in Docker containers; they need <a href={settingsHref('sandbox')}>Settings → Sandbox</a> = Docker.</p>
    <Button variant="primary" icon="plus" onclick={() => openDialog()}>Add host</Button>
  </div>

  <Table
    caption="Hosts"
    {columns}
    rows={hostList.data && hosts}
    key={(row: Host) => row.id}
    loading={hostList.loading}
    error={hostList.error}
    onretry={() => hostList.reload()}
    empty={{ icon: 'hard-drive', title: 'No hosts yet', description: 'Add a machine that brr reaches over SSH to run more browsers.' }}
  >
    {#snippet row(host: Host)}
      {@const shown = status(host)}
      <td class="truncate" title={host.name}>{host.name}</td>
      <td class="truncate" title={host.docker_url}>{host.docker_url}</td>
      {@const detail = checking.has(host.id) ? '' : statusDetail(host)}
      <td class="status" title={detail ? `${shown.label}${detail}` : undefined}>
        <span class="line">
          <StatusDot tone={shown.tone} label={shown.label} pulse={checking.has(host.id)} />
          {#if detail}<span class="cause">{detail}</span>{/if}
        </span>
      </td>
      <td class="num right">{memory(host)}</td>
      <td class="num right">{host.status.running} / {host.max_instances}</td>
      <td>{image(host)}</td>
      <td>
        <div class="actions">
          <Button size="sm" loading={checking.has(host.id)} onclick={() => check(host)}>Check now</Button>
          <Menu
            label="Actions for {host.name}"
            items={[
              { label: 'Edit…', onselect: () => openDialog(host) },
              { label: 'Delete…', tone: 'danger', onselect: () => remove(host) },
            ]}
          />
        </div>
      </td>
    {/snippet}
  </Table>
</div>

<Dialog bind:open={dialogOpen} title={editing ? 'Edit host' : 'Add host'} dismissible={!saving}>
  <form id="host-form" onsubmit={save} novalidate>
    <div class="fields">
      <Input label="Name" bind:value={name} required placeholder="build-box" error={errors.name} autocomplete="off" />
      <Input
        label="SSH URL"
        bind:value={dockerUrl}
        required
        placeholder="ssh://user@host"
        hint="brr uses the daemon user's SSH keys; the user must be allowed to run docker"
        error={errors.docker_url}
        autocomplete="off"
      />
      <Input label="Max browsers" type="number" min={1} max={MAX_BROWSERS} step={1} bind:value={maxInstances} required error={errors.max_instances} />
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={() => (dialogOpen = false)} disabled={saving}>Cancel</Button>
    <Button variant="primary" type="submit" form="host-form" loading={saving}>{editing ? 'Save host' : 'Add host'}</Button>
  {/snippet}
</Dialog>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .note,
  .cause {
    margin: 0;
    color: var(--text-secondary);
  }

  .note a {
    text-decoration: underline;
  }

  .line {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .line :global(.status) {
    flex-shrink: 0;
  }

  .cause {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
</style>
