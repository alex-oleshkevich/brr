<script lang="ts">
  import { untrack } from 'svelte';
  import { goto } from '$app/navigation';
  import { ApiError, api, unwrap } from '$lib/api/client';
  import { events } from '$lib/api/events.svelte';
  import { liveQuery } from '$lib/api/live.svelte';
  import type { components } from '$lib/api/schema';
  import { absolute, bytes } from '$lib/format';
  import Button from '$lib/ui/Button.svelte';
  import Card from '$lib/ui/Card.svelte';
  import Dialog from '$lib/ui/Dialog.svelte';
  import Input from '$lib/ui/Input.svelte';
  import Menu from '$lib/ui/Menu.svelte';
  import Progress from '$lib/ui/Progress.svelte';
  import SegmentedControl from '$lib/ui/SegmentedControl.svelte';
  import Select from '$lib/ui/Select.svelte';
  import Table from '$lib/ui/Table.svelte';
  import { confirm } from '$lib/ui/confirm.svelte';
  import { toast } from '$lib/ui/toast.svelte';
  import { fieldError } from '../form';
  import { profileNameError, restoredName } from '../restore';

  type Backup = components['schemas']['Backup'];
  type Profile = components['schemas']['Profile'];

  let { profile, onsaved }: { profile: Profile; onsaved: (next: Profile) => void } = $props();

  const profileId = untrack(() => profile.id);

  const DAYS = [
    { value: 'mon', label: 'Monday' },
    { value: 'tue', label: 'Tuesday' },
    { value: 'wed', label: 'Wednesday' },
    { value: 'thu', label: 'Thursday' },
    { value: 'fri', label: 'Friday' },
    { value: 'sat', label: 'Saturday' },
    { value: 'sun', label: 'Sunday' },
  ];

  function parse(schedule: string | null | undefined) {
    const daily = schedule?.match(/^daily@(\d\d:\d\d)$/);
    if (daily) return { mode: 'daily', day: 'mon', time: daily[1] };
    const weekly = schedule?.match(/^weekly@([a-z]{3})@(\d\d:\d\d)$/);
    if (weekly) return { mode: 'weekly', day: weekly[1], time: weekly[2] };
    return { mode: 'off', day: 'mon', time: '03:00' };
  }

  const initial = untrack(() => parse(profile.backup_schedule));
  let mode = $state(initial.mode);
  let day = $state(initial.day);
  let time = $state(initial.time);
  let keep = $state<number | null>(untrack(() => profile.backup_keep));
  let scheduleBusy = $state(false);
  let scheduleErrors = $state<Record<string, string>>({});

  // The time input yields 24-hour "HH:MM" whatever the locale, which is the schedule's own format.
  const schedule = $derived(mode === 'daily' ? `daily@${time}` : mode === 'weekly' ? `weekly@${day}@${time}` : null);
  const scheduleDirty = $derived(schedule !== (profile.backup_schedule ?? null) || (keep ?? profile.backup_keep) !== profile.backup_keep);

  async function saveSchedule() {
    if (scheduleBusy) return;
    scheduleBusy = true;
    scheduleErrors = {};
    try {
      const next = await unwrap(
        api.PATCH('/v1/profiles/{profile}', {
          params: { path: { profile: profileId } },
          body: { profile: profileId, backup_schedule: schedule, backup_keep: keep ?? profile.backup_keep },
        }),
      );
      onsaved(next);
      toast.success('Backup schedule saved');
    } catch (e) {
      if (e instanceof ApiError && e.code === 'InvalidArgument' && typeof e.details?.field === 'string') scheduleErrors = { [e.details.field]: e.message };
      else toast.apiError(e, "Couldn't save the schedule");
    } finally {
      scheduleBusy = false;
    }
  }

  const settings = liveQuery(() => unwrap(api.GET('/v1/settings')), { kinds: ['settings.changed'] });
  const hasPassphrase = $derived(settings.data?.secrets['backup.passphrase']?.set === true);

  const list = liveQuery(
    async () => (await unwrap(api.GET('/v1/profiles/{profile}/backups', { params: { path: { profile: profileId } } }))).backups,
    { kinds: ['backup.completed'], match: (e) => (e.data as { profile_id?: string } | null)?.profile_id === profileId },
  );

  // Passphrase dialog.
  let passOpen = $state(false);
  let pass1 = $state('');
  let pass2 = $state('');
  let passBusy = $state(false);
  let passError = $state<string | null>(null);
  const passMismatch = $derived(pass2 !== '' && pass1 !== pass2);

  function openPassphrase() {
    pass1 = '';
    pass2 = '';
    passError = null;
    passOpen = true;
  }

  async function setPassphrase(event: SubmitEvent) {
    event.preventDefault();
    if (passBusy || pass1 === '' || pass1 !== pass2) return;
    passBusy = true;
    passError = null;
    try {
      await unwrap(api.PATCH('/v1/settings', { body: { 'backup.passphrase': pass1 } }));
      passOpen = false;
      toast.success('Backup passphrase set');
      settings.reload();
    } catch (e) {
      passError = e instanceof ApiError ? e.message : "Couldn't set the passphrase.";
    } finally {
      passBusy = false;
    }
  }

  // backup.create both fails and publishes backup.skipped or backup.failed for the same outcome. While this
  // page has one in flight, its result is toasted from the API response and the events are ignored;
  // scheduled backups still toast from their events.
  let backingUp = $state(false);

  $effect(() =>
    events.on(['backup.skipped', 'backup.failed'], (e) => {
      const data = e.data as { profile_id?: string; error?: string } | null;
      if (data?.profile_id !== profileId || backingUp) return;
      if (e.kind === 'backup.skipped') toast.show({ tone: 'warning', title: 'Skipped: the profile is running' });
      else toast.show({ tone: 'danger', title: 'Backup failed', description: data?.error });
    }),
  );

  async function backUp() {
    if (backingUp) return;
    backingUp = true;
    try {
      await unwrap(api.POST('/v1/profiles/{profile}/backups', { params: { path: { profile: profileId } }, body: {} }));
      toast.success('Backup created');
      list.reload();
    } catch (e) {
      if (e instanceof ApiError && e.code === 'Conflict' && e.details.reason === 'profile_running') {
        toast.show({ tone: 'warning', title: 'Stop the profile first. Running profiles can\'t be backed up.' });
      } else {
        toast.apiError(e, "Couldn't create the backup");
      }
    } finally {
      backingUp = false;
    }
  }

  // Restore dialog.
  let restoring = $state<Backup | null>(null);
  let restoreOpen = $state(false);
  let restoreName = $state('');
  let restoreBusy = $state(false);
  let restoreError = $state<string | null>(null);
  const restoreNameError = $derived(profileNameError(restoreName.trim()));

  function openRestore(backup: Backup) {
    restoring = backup;
    restoreName = restoredName(profile.name, backup.created_at);
    restoreError = null;
    restoreOpen = true;
  }

  async function restore(event: SubmitEvent) {
    event.preventDefault();
    if (!restoring || restoreBusy || restoreNameError) return;
    restoreBusy = true;
    restoreError = null;
    try {
      const out = await unwrap(
        api.POST('/v1/backups/{backup_id}/restore', { params: { path: { backup_id: restoring.id } }, body: { name: restoreName.trim() } }),
      );
      restoreOpen = false;
      toast.success(`Restored as ${out.profile.name}`);
      await goto(`/profiles/${out.profile.id}`);
    } catch (e) {
      if (e instanceof ApiError && (e.code === 'Conflict' || (e.code === 'InvalidArgument' && e.details?.field === 'name'))) restoreError = e.message;
      else toast.apiError(e, "Couldn't restore the backup");
    } finally {
      restoreBusy = false;
    }
  }

  async function remove(backup: Backup) {
    const ok = await confirm({
      title: 'Delete this backup?',
      description: `The archive from ${absolute(backup.created_at)} is deleted. This can't be undone.`,
      confirmLabel: 'Delete backup',
      tone: 'danger',
    });
    if (!ok) return;
    try {
      await unwrap(api.DELETE('/v1/backups/{backup_id}', { params: { path: { backup_id: backup.id } } }));
      toast.success('Backup deleted');
      list.reload();
    } catch (e) {
      toast.apiError(e, "Couldn't delete the backup");
    }
  }

  const backendLabel = (backend: string) => (backend === 'local' ? 'Local' : backend.toUpperCase());
  const locked = $derived(!hasPassphrase);
</script>

<div class="stack">
  {#if profile.kind === 'ephemeral'}
    <Card title="Backups"><p class="field-hint">Ephemeral profiles keep no data, so there is nothing to back up.</p></Card>
  {:else}
    {#if settings.data && !hasPassphrase}
      <div class="callout" role="status">
        <div>
          <strong>Set a backup passphrase to enable backups</strong>
          <p>Backups are encrypted with it. You need it to restore them.</p>
        </div>
        <Button variant="primary" onclick={openPassphrase}>Set passphrase…</Button>
      </div>
    {/if}

    <Card title="Schedule" description="Back up this profile automatically. Saved on its own with the button below.">
      <div class="fields">
        <div class="field">
          <span class="field-label">Schedule</span>
          <SegmentedControl
            label="Schedule"
            bind:value={mode}
            disabled={locked}
            options={[
              { value: 'off', label: 'Off' },
              { value: 'daily', label: 'Daily' },
              { value: 'weekly', label: 'Weekly' },
            ]}
          />
          {#if fieldError(scheduleErrors, 'backup_schedule')}<p class="field-error">{fieldError(scheduleErrors, 'backup_schedule')}</p>{/if}
        </div>
        {#if mode !== 'off'}
          <div class="pair">
            {#if mode === 'weekly'}<Select label="Day" bind:value={day} options={DAYS} disabled={locked} />{/if}
            <Input label="At" type="time" bind:value={time} disabled={locked} hint="Local time." />
          </div>
        {/if}
        <div class="pair">
          <Input label="Keep copies" type="number" min={1} step={1} bind:value={keep} disabled={locked} error={fieldError(scheduleErrors, 'backup_keep')} />
        </div>
        <div><Button size="sm" loading={scheduleBusy} disabled={locked || !scheduleDirty} onclick={saveSchedule}>Save schedule</Button></div>
      </div>
    </Card>

    <Card title="Backups" description="Logins aren't included in backups; re-link them after restoring.">
      <div class="fields">
        <Table
          caption="Backups"
          columns={[
            { key: 'created', label: 'Created', width: '50%' },
            { key: 'size', label: 'Size', width: '20%', align: 'right' },
            { key: 'backend', label: 'Backend', width: '20%' },
            { key: 'actions', label: 'Actions', width: '48px', srOnly: true },
          ]}
          rows={list.data}
          key={(row: Backup) => row.id}
          loading={list.loading}
          error={list.error}
          onretry={() => list.reload()}
          empty={hasPassphrase
            ? { title: 'No backups yet', description: 'Back up now, or set a schedule above.' }
            : { title: 'No backups yet', description: 'Set a backup passphrase to enable backups.' }}
        >
          {#snippet row(backup: Backup)}
            <td>{absolute(backup.created_at)}</td>
            <td class="num right">{bytes(backup.size_bytes)}</td>
            <td>{backendLabel(backup.backend)}</td>
            <td>
              <Menu
                label="Actions for the backup from {absolute(backup.created_at)}"
                items={[
                  { label: 'Restore as new profile…', onselect: () => openRestore(backup) },
                  { label: 'Delete…', tone: 'danger', onselect: () => remove(backup) },
                ]}
              />
            </td>
          {/snippet}
        </Table>
        <div class="run">
          <Button variant="primary" size="sm" loading={backingUp} disabled={!hasPassphrase} onclick={backUp}>Back up now</Button>
          {#if backingUp}
            <div class="progress" role="status">
              <Progress value={null} label="Backing up {profile.name}" />
              <span>Backing up {profile.name}: archiving, encrypting and uploading. This can take a minute.</span>
            </div>
          {/if}
        </div>
      </div>
    </Card>
  {/if}
</div>

<Dialog bind:open={passOpen} title="Set a backup passphrase" description="Backups are encrypted with this passphrase. Keep it safe: without it a backup can't be restored." dismissible={!passBusy}>
  <form id="passphrase-form" onsubmit={setPassphrase}>
    <div class="fields">
      <Input label="Passphrase" type="password" bind:value={pass1} required autocomplete="new-password" />
      <Input label="Confirm passphrase" type="password" bind:value={pass2} required autocomplete="new-password" error={passMismatch ? "The passphrases don't match." : passError} />
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={() => (passOpen = false)} disabled={passBusy}>Cancel</Button>
    <Button variant="primary" type="submit" form="passphrase-form" loading={passBusy} disabled={pass1 === '' || pass1 !== pass2}>Set passphrase</Button>
  {/snippet}
</Dialog>

<Dialog
  bind:open={restoreOpen}
  title="Restore as new profile"
  description="Creates a new profile from the backup of {restoring ? absolute(restoring.created_at) : ''}. The original is not changed."
  dismissible={!restoreBusy}
>
  <form id="restore-form" onsubmit={restore}>
    <div class="fields">
      <Input label="New profile name" bind:value={restoreName} required error={restoreName.trim() ? (restoreNameError ?? restoreError) : null} disabled={restoreBusy} />
      <p class="field-hint">Logins aren't included in backups; re-link them after restoring.</p>
      {#if restoreBusy}
        <div class="progress" role="status">
          <Progress value={null} label="Restoring" />
          <span>Restoring: downloading, decrypting and creating the profile. This can take up to a minute, so keep this window open.</span>
        </div>
      {/if}
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={() => (restoreOpen = false)} disabled={restoreBusy}>Cancel</Button>
    <Button variant="primary" type="submit" form="restore-form" loading={restoreBusy} disabled={!!restoreNameError}>Restore</Button>
  {/snippet}
</Dialog>

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

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .pair {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-3);
  }

  .callout {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4);
    background: var(--warning-bg);
    border: 1px solid var(--warning-border);
    border-radius: var(--radius-lg);
  }

  .callout p {
    margin: 0;
    color: var(--text-secondary);
  }

  .run {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1;
    color: var(--text-secondary);
    font-size: var(--text-sm-size);
  }
</style>
