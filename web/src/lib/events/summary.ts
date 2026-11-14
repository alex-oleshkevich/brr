import type { BrrEvent } from '$lib/api/events.svelte';
import { session } from '$lib/api/session';
import { SOURCE_NAMES } from '$lib/builds/labels';
import { clock } from '$lib/clock.svelte';
import { bytes, usd } from '$lib/format';
import { names } from './names.svelte';

const MAX_LENGTH = 160;

type Data = Record<string, unknown>;

// The daemon stamps each event with the client whose request caused it; null for daemon-initiated ones.
const isYou = (clientId: unknown) => typeof clientId === 'string' && session.uiClientIds.has(clientId);
const byYou = (e: BrrEvent) => isYou(e.actor_client_id);
const text = (value: unknown) => (typeof value === 'string' ? value : String(value));
const scalar = (value: unknown) => ['string', 'number', 'boolean'].includes(typeof value);
const sentence = (value: string) => value.charAt(0).toUpperCase() + value.slice(1);
const words = (value: unknown) => text(value).replaceAll('_', ' ');

const END_REASONS: Record<string, string> = {
  stop: 'Stopped',
  shutdown: 'The daemon shut down',
  last_tab_closed: 'Its last tab was closed',
  cancel: 'Cancelled before it started',
  queue_timeout: 'Waited too long in the queue',
  launch_error: "The browser couldn't start",
  start_timeout: 'The browser took too long to start',
  crashed: 'The browser crashed',
  daemon_restart: 'The daemon restarted',
  evicted: 'Stopped to make room for another browser',
  memory_pressure: 'Stopped because memory was low',
  reaped: 'Stopped after sitting idle',
};

// Instance trigger -> why it moved to its new state, for the triggers a person would care about.
const TRIGGERS: Record<string, string> = {
  stop: 'stopped',
  idle_timeout: 'idle too long',
  reap: 'reaped for sitting idle',
  evict: 'evicted to make room',
  memory_pressure: 'low memory',
  last_tab_closed: 'last tab closed',
  launch_error: 'launch failed',
  start_timeout: 'start timed out',
  unexpected_exit: 'the browser exited',
  queue_timeout: 'queue timeout',
  cancel: 'cancelled',
  connection_lost: 'connection lost',
};

const TAB_REASONS: Record<string, string> = {
  client: 'closed by an agent',
  page: 'closed by the page',
  instance_stopped: 'its browser stopped',
  reaped: 'idle too long',
  daemon_restart: 'the daemon restarted',
};

const ADMISSIONS: Record<string, string> = {
  admitted: 'was cleared to start',
  queued: 'is waiting for a free slot',
  rejected: "couldn't start: no room for it",
  evicted: 'was stopped to make room for another browser',
};

const RELEASE_REASONS: Record<string, string> = {
  released: 'handed back',
  client_ended: 'the agent disconnected',
  tab_closed: 'the tab closed',
  instance_stopped: 'the browser stopped',
  instance_crashed: 'the browser crashed',
  daemon_restart: 'the daemon restarted',
  transferred: 'taken over',
};

const HUMAN_NEEDS: Record<string, string> = {
  login: 'a login',
  captcha: 'a captcha solved',
  dialog: 'a dialog answered',
};

// The daemon words a death by its signal; a person wants "killed".
export const plainError = (error: string): string => error.replace(/killed by signal 9\b/, 'killed').replace(/ by signal (\d+)/, ' (signal $1)');

export const endReason = (reason: string): string => END_REASONS[reason] ?? sentence(words(reason));

// "1:30 left"; a deadline already past reads "overdue".
export function countdown(deadline: number, now = clock.now): string {
  const seconds = Math.ceil((deadline - now) / 1000);
  if (seconds <= 0) return 'overdue';
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')} left`;
}

function host(url: string): string {
  try {
    const { hostname, pathname } = new URL(url);
    if (!hostname) return url;
    return hostname + (pathname === '/' ? '' : pathname);
  } catch {
    return url;
  }
}

const build = (data: Data) => `${SOURCE_NAMES[data.source as keyof typeof SOURCE_NAMES] ?? text(data.source ?? '')} ${data.version ?? ''}`.trim();

function describe(e: BrrEvent, data: Data): string | null {
  const instance = () => names.instance(text(data.instance_id));
  const tab = () => names.tab(text(data.tab_id));
  const profile = () => names.profile(text(data.profile_id));

  switch (e.kind) {
    case 'instance.state_changed': {
      const trigger = TRIGGERS[text(data.trigger)];
      const to = text(data.to);
      // The daemon's reason is the cause a person is looking for ("proxy unhealthy", "killed by signal 9").
      const cause = data.reason ? `: ${plainError(text(data.reason))}` : '';
      if (to === 'failed') return `${instance()} failed to start${cause}`;
      if (to === 'crashed') return `${instance()} crashed${cause}`;
      return `${instance()} ${to === 'running' ? 'is running' : to === 'queued' ? 'is waiting to start' : words(to)}${trigger ? ` (${trigger})` : ''}`;
    }
    case 'instance.queued':
      return `${instance()} is waiting to start (position ${data.queue_pos})`;
    case 'instance.reaping_warning':
      return `${instance()} will stop for inactivity (${countdown(Number(data.grace_ends_at))})`;
    case 'tab.opened':
      return `Opened ${host(text(data.url))} in ${instance()}`;
    case 'tab.closed':
      return `${tab()} closed in ${instance()} (${data.reason === 'client' && byYou(e) ? 'closed by you' : (TAB_REASONS[text(data.reason)] ?? words(data.reason))})`;
    case 'tab.crashed':
      return `${tab()} crashed in ${instance()}`;
    case 'tab.reaping_warning':
      return `${tab()} in ${instance()} will close for inactivity (${countdown(Number(data.grace_ends_at))})`;
    case 'tab.awaiting_human':
      return `${tab()} in ${instance()} needs ${HUMAN_NEEDS[text(data.reason)] ?? 'you'}`;
    case 'tab.navigated':
      return `${tab()} went to ${host(text(data.url))}`;
    case 'client.created':
      return `${text(data.label || 'An agent')} connected`;
    case 'client.ended':
      if (isYou(data.client_id)) return 'A web session ended';
      return `An agent ${text(data.reason) === 'idle' ? 'timed out' : text(data.reason) === 'daemon_restart' ? 'was dropped by a daemon restart' : 'disconnected'}`;
    case 'lease.acquired':
      return `${byYou(e) ? 'You' : 'An agent'} took ${data.tab_id ? 'a tab' : 'control'} in ${instance()}${data.exclusive ? ' exclusively' : ''}`;
    case 'lease.released':
      return `${data.tab_id ? 'A tab' : 'Control'} in ${instance()} was given back (${RELEASE_REASONS[text(data.reason)] ?? words(data.reason)})`;
    case 'lease.transferred':
      return isYou(data.to_client_id)
        ? `You took over a tab in ${instance()}`
        : `A tab in ${instance()} was handed back to an agent`;
    case 'admission.decided':
      return `${instance()} ${ADMISSIONS[text(data.outcome)] ?? words(data.outcome)}`;
    case 'system.memory_pressure':
      return `Low memory (${data.free_ram_mb} MB free) · ${words(data.action)}`;
    case 'build.installed':
      return `${build(data)} installed`;
    case 'build.install_failed':
      return `${build(data)} failed to install${data.error ? `: ${data.error}` : ''}`;
    case 'build.removed':
      return `${build(data)} removed`;
    case 'build.channel_moved':
      return `${SOURCE_NAMES[data.source as keyof typeof SOURCE_NAMES] ?? text(data.source)} ${text(data.channel)} moved to ${text(data.to)}`;
    case 'profile.created':
    case 'profile.updated':
    case 'profile.deleted':
      return `Profile ${text(data.name)} ${e.kind === 'profile.updated' ? 'changed' : e.kind.split('.')[1]}`;
    case 'extension.updated':
      return `An extension updated to ${text(data.to)}`;
    case 'download.started':
    case 'download.completed':
    case 'download.cancelled':
      return `${text(data.filename)} ${e.kind.split('.')[1]} · ${bytes(Number(data.size))}`;
    case 'artifact.created':
      return `${sentence(text(data.kind))} saved (${bytes(Number(data.size))})`;
    case 'artifact.deleted':
      return `${sentence(text(data.kind))} deleted (${bytes(Number(data.size))})`;
    case 'retention.sweep_completed':
      return `${data.dry_run ? 'Cleanup preview' : 'Cleanup'} · ${bytes(Number(data.bytes))}`;
    case 'backup.completed':
      return `Backed up ${profile()}`;
    case 'backup.failed':
      return `Backup of ${profile()} failed${data.error ? `: ${data.error}` : ''}`;
    case 'backup.skipped':
      return `Backup of ${profile()} skipped`;
    case 'captcha.detected':
      return `${sentence(words(data.type))} captcha on ${text(data.site)}`;
    case 'captcha.solved':
      return `Solved a ${words(data.type)} captcha on ${text(data.site)} (${usd(Number(data.cost_usd))})`;
    case 'captcha.failed':
      return `Couldn't solve the ${words(data.type)} captcha on ${text(data.site)}`;
    case 'captcha.budget_exhausted':
      return `Spent ${usd(Number(data.spent_usd))} on ${data.solves} solves today`;
    case 'login.performed':
      return `Login ${data.ok ? 'succeeded' : 'failed'} in ${instance()}`;
    case 'login.check_result':
      return `${text(data.site)} · ${profile()}: ${text(data.status)}`;
    case 'login.relogin_suspended':
      return `Re-login on ${text(data.site)} paused after ${data.failures} failures (${profile()})`;
    case 'proxy.health':
      return data.healthy
        ? `Proxy healthy${data.exit_ip ? ` · ${data.exit_ip}` : ''}${data.country ? ` (${data.country})` : ''}`
        : `Proxy unhealthy${data.error ? `: ${text(data.error)}` : ''}`;
    case 'limits.changed':
    case 'settings.changed':
      return `${e.kind === 'limits.changed' ? 'Capacity' : 'Settings'} changed: ${(data.keys as string[]).join(', ')}`;
    default:
      return null;
  }
}

// Events a person starts from the UI; the rest (idle timeouts, crashes, agent calls) have no "you".
const UI_ACTIONS = new Set(['tab.opened', 'profile.created', 'profile.updated', 'profile.deleted', 'build.installed', 'build.removed', 'artifact.deleted', 'retention.sweep_completed', 'backup.completed', 'settings.changed', 'limits.changed', 'lease.released']);
const UI_TRIGGERS = new Set(['launch', 'stop', 'cancel']);

function uiInitiated(e: BrrEvent, data: Data): boolean {
  const candidate = UI_ACTIONS.has(e.kind) || (e.kind === 'instance.state_changed' && UI_TRIGGERS.has(text(data.trigger)));
  return candidate && byYou(e);
}

export function summarize(e: BrrEvent): string {
  const data = (e.data ?? {}) as Data;
  let line: string | null = null;
  try {
    line = describe(e, data);
  } catch {
    // A payload that doesn't match the known shape falls through to the generic line.
  }

  line ??= Object.entries(data)
    .filter(([key, value]) => !key.endsWith('_id') && (scalar(value) || (Array.isArray(value) && value.every(scalar))))
    .slice(0, 3)
    .map(([key, value]) => `${words(key)} ${Array.isArray(value) ? value.join(', ') : value}`)
    .join(' · ');

  if (uiInitiated(e, data)) line += ' · by you';

  return line.length > MAX_LENGTH ? line.slice(0, MAX_LENGTH - 1) + '…' : line;
}
