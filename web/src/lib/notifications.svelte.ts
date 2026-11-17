import { ApiError, api, unwrap } from './api/client';
import { events, type BrrEvent } from './api/events.svelte';
import { session } from './api/session';
import { attention, hostOf, REASON_VERBS } from './attention/attention.svelte';
import { names } from './events/names.svelte';
import { summarize } from './events/summary';
import { toast, type ToastTone } from './ui/toast.svelte';

const READ_KEY = 'brr.notifications.read_seq';
const LIMIT = 20;
const DAY_MS = 86_400_000;
const STATE_EVENTS_LIMIT = 500;

export const NOTIFICATION_KINDS = ['tab.awaiting_human', 'build.install_failed', 'captcha.budget_exhausted', 'backup.failed', 'login.relogin_suspended'];

const titles: Record<string, { title: string; tone: ToastTone }> = {
  'tab.awaiting_human': { title: 'A tab needs you', tone: 'warning' },
  'build.install_failed': { title: 'Browser version install failed', tone: 'danger' },
  'captcha.budget_exhausted': { title: 'Captcha budget used up', tone: 'warning' },
  'backup.failed': { title: 'Backup failed', tone: 'danger' },
  'login.relogin_suspended': { title: 'Auto re-login suspended', tone: 'warning' },
};

// instance.state_changed is a notification only when the browser ended badly.
// An attached browser has no profile; one that never started is a failed attach, which the dialog already reports.
const isFailure = (e: BrrEvent) => {
  const data = e.data as { to?: string; profile_id?: string | null } | null;
  if (e.kind !== 'instance.state_changed' || !['crashed', 'failed'].includes(data?.to ?? '')) return false;
  return !(data?.to === 'failed' && !data.profile_id);
};

export const notificationTitle = (e: BrrEvent) => {
  if (isFailure(e)) {
    return (e.data as { to: string }).to === 'crashed'
      ? { title: 'A browser crashed', tone: 'danger' as const }
      : { title: "A browser couldn't start", tone: 'danger' as const };
  }
  return titles[e.kind] ?? { title: e.kind, tone: 'neutral' as const };
};

export async function notificationHref(e: BrrEvent): Promise<string | null> {
  if (e.kind === 'build.install_failed') return '/versions';
  if (e.kind === 'captcha.budget_exhausted') return '/?settings=captcha';
  if (isFailure(e)) return `/browsers/${(e.data as { instance_id: string }).instance_id}`;
  if (e.kind === 'login.relogin_suspended') {
    const profileId = (e.data as { profile_id?: string } | null)?.profile_id;
    return profileId ? `/profiles/${profileId}#logins` : null;
  }
  if (e.kind === 'backup.failed') {
    const profileId = (e.data as { profile_id?: string } | null)?.profile_id;
    return profileId ? `/profiles/${profileId}#backups` : null;
  }
  if (e.kind !== 'tab.awaiting_human') return null;

  const tabId = (e.data as { tab_id: string }).tab_id;
  const tab = await unwrap(api.GET('/v1/tabs/{tab_id}', { params: { path: { tab_id: tabId } } }));
  return `/browsers/${tab.instance_id}?tab=${tabId}`;
}

class Notifications {
  items = $state<BrrEvent[]>([]);
  error = $state<ApiError | null>(null);
  private readSeq = $state(Number(localStorage.getItem(READ_KEY) ?? 0));
  // An awaiting-human item is open only while its tab still waits; once the person is done, it times out or the tab closes, it is resolved.
  active = $derived(this.items.filter((e) => !this.isResolved(e)));
  unread = $derived(this.active.filter((e) => e.seq > this.readSeq).length);
  needsYou = $derived(attention.needsYou.length > 0);

  async start(): Promise<void> {
    // Subscribe before loading so nothing falls in the gap; seq dedupes the overlap.
    events.on([...NOTIFICATION_KINDS, 'instance.state_changed'], (e) => {
      if (e.kind === 'instance.state_changed' && !isFailure(e)) return;
      if (this.items.some((item) => item.seq === e.seq)) return;
      this.items = [e, ...this.items].slice(0, LIMIT);
      // A launch started from this UI reports its own failure; a second toast would say the same thing.
      const reported = isFailure(e) && (e.data as { to: string }).to === 'failed' && session.uiClientIds.has(e.actor_client_id ?? '');
      if (e.kind === 'tab.awaiting_human' || (isFailure(e) && !reported)) {
        const { title, tone } = notificationTitle(e);
        void names.preload(e.data as Record<string, unknown> | null).then(() => toast.show({ tone, title, description: summarize(e) }));
      }
    });

    await this.load();
  }

  async load(): Promise<void> {
    try {
      const [{ events: initial }, { events: states }] = await Promise.all([
        unwrap(api.GET('/v1/events', { params: { query: { kinds: NOTIFICATION_KINDS.join(','), order: 'desc', limit: LIMIT } } })),
        unwrap(
          api.GET('/v1/events', {
            params: { query: { kinds: 'instance.state_changed', from_ts: Date.now() - DAY_MS, order: 'desc', limit: STATE_EVENTS_LIMIT } },
          }),
        ),
      ]);
      const seen = new Set(this.items.map((e) => e.seq));
      this.items = [...this.items, ...[...initial, ...states.filter(isFailure)].filter((e) => !seen.has(e.seq))]
        .sort((a, b) => b.seq - a.seq)
        .slice(0, LIMIT);
      this.error = null;
    } catch (e) {
      this.error = e instanceof ApiError ? e : null;
    }
  }

  markAllRead(): void {
    this.readSeq = Math.max(0, ...this.items.map((e) => e.seq), this.readSeq);
    localStorage.setItem(READ_KEY, String(this.readSeq));
  }

  isUnread(e: BrrEvent): boolean {
    return e.seq > this.readSeq && !this.isResolved(e);
  }

  isResolved(e: BrrEvent): boolean {
    if (e.kind !== 'tab.awaiting_human') return false;
    const tabId = (e.data as { tab_id: string }).tab_id;
    return attention.needsYouLoaded && !attention.needsYou.some((item) => item.tabId === tabId);
  }

  // What the notification says; an awaiting-human one names the site and what is needed, and drops the countdown that goes stale.
  text(e: BrrEvent): string {
    if (e.kind !== 'tab.awaiting_human') return summarize(e);
    const data = e.data as { tab_id: string; reason: keyof typeof REASON_VERBS };
    const open = attention.needsYou.find((item) => item.tabId === data.tab_id);
    return open ? `${open.agent} needs you to ${REASON_VERBS[data.reason]} on ${hostOf(open.url)}` : summarize(e);
  }
}

export const notifications = new Notifications();
