// Every event kind: its plain-language label, whether it is only streamed (never stored), and its Activity category.
// The raw kind is shown only in an event's expanded JSON.
export type Category = 'browsers' | 'tabs' | 'agents' | 'capacity' | 'setup' | 'files' | 'logins';

export const KINDS: { kind: string; label: string; transient: boolean; category: Category }[] = [
  { kind: 'instance.state_changed', label: 'Browser changed state', transient: false, category: 'browsers' },
  { kind: 'instance.queued', label: 'Browser waiting to start', transient: false, category: 'browsers' },
  { kind: 'instance.reaping_warning', label: 'Browser about to close', transient: false, category: 'browsers' },
  { kind: 'tab.opened', label: 'Tab opened', transient: false, category: 'tabs' },
  { kind: 'tab.closed', label: 'Tab closed', transient: false, category: 'tabs' },
  { kind: 'tab.state_changed', label: 'Tab changed state', transient: true, category: 'tabs' },
  { kind: 'tab.crashed', label: 'Tab crashed', transient: false, category: 'tabs' },
  { kind: 'tab.reaping_warning', label: 'Tab about to close', transient: false, category: 'tabs' },
  { kind: 'tab.awaiting_human', label: 'Agent asked for help', transient: false, category: 'tabs' },
  { kind: 'tab.navigated', label: 'Tab navigated', transient: true, category: 'tabs' },
  { kind: 'client.created', label: 'Agent connected', transient: false, category: 'agents' },
  { kind: 'client.ended', label: 'Agent disconnected', transient: false, category: 'agents' },
  { kind: 'lease.acquired', label: 'Control taken', transient: false, category: 'agents' },
  { kind: 'lease.released', label: 'Control given back', transient: false, category: 'agents' },
  { kind: 'lease.transferred', label: 'Tab taken over or handed back', transient: false, category: 'agents' },
  { kind: 'admission.decided', label: 'Capacity check', transient: false, category: 'capacity' },
  { kind: 'system.memory_pressure', label: 'Low memory', transient: false, category: 'capacity' },
  { kind: 'limits.changed', label: 'Capacity changed', transient: false, category: 'capacity' },
  { kind: 'build.install_progress', label: 'Browser version installing', transient: true, category: 'setup' },
  { kind: 'build.installed', label: 'Browser version installed', transient: false, category: 'setup' },
  { kind: 'build.install_failed', label: 'Browser version install failed', transient: false, category: 'setup' },
  { kind: 'build.removed', label: 'Browser version removed', transient: false, category: 'setup' },
  { kind: 'build.channel_moved', label: 'Browser version updated', transient: false, category: 'setup' },
  { kind: 'profile.created', label: 'Profile created', transient: false, category: 'setup' },
  { kind: 'profile.updated', label: 'Profile changed', transient: false, category: 'setup' },
  { kind: 'profile.deleted', label: 'Profile deleted', transient: false, category: 'setup' },
  { kind: 'extension.updated', label: 'Extension updated', transient: false, category: 'setup' },
  { kind: 'proxy.health', label: 'Proxy checked', transient: false, category: 'setup' },
  { kind: 'settings.changed', label: 'Settings changed', transient: false, category: 'setup' },
  { kind: 'download.started', label: 'Download started', transient: true, category: 'files' },
  { kind: 'download.completed', label: 'Download finished', transient: false, category: 'files' },
  { kind: 'download.cancelled', label: 'Download cancelled', transient: false, category: 'files' },
  { kind: 'artifact.created', label: 'File saved', transient: false, category: 'files' },
  { kind: 'artifact.deleted', label: 'File deleted', transient: false, category: 'files' },
  { kind: 'retention.sweep_completed', label: 'Cleanup ran', transient: false, category: 'files' },
  { kind: 'backup.completed', label: 'Backup finished', transient: false, category: 'files' },
  { kind: 'backup.failed', label: 'Backup failed', transient: false, category: 'files' },
  { kind: 'backup.skipped', label: 'Backup skipped', transient: false, category: 'files' },
  { kind: 'captcha.detected', label: 'Captcha appeared', transient: false, category: 'logins' },
  { kind: 'captcha.solved', label: 'Captcha solved', transient: false, category: 'logins' },
  { kind: 'captcha.failed', label: 'Captcha failed', transient: false, category: 'logins' },
  { kind: 'captcha.budget_exhausted', label: 'Captcha budget used up', transient: false, category: 'logins' },
  { kind: 'bitwarden.synced', label: 'Bitwarden synced', transient: false, category: 'logins' },
  { kind: 'bitwarden.sync_failed', label: 'Bitwarden sync failed', transient: false, category: 'logins' },
  { kind: 'login.performed', label: 'Login attempted', transient: false, category: 'logins' },
  { kind: 'login.check_result', label: 'Login checked', transient: false, category: 'logins' },
  { kind: 'login.relogin_suspended', label: 'Auto re-login paused', transient: false, category: 'logins' },
];

export const CATEGORIES: { id: Category; label: string }[] = [
  { id: 'browsers', label: 'Browsers' },
  { id: 'tabs', label: 'Tabs' },
  { id: 'agents', label: 'Agents and control' },
  { id: 'capacity', label: 'Capacity' },
  { id: 'setup', label: 'Setup and settings' },
  { id: 'files', label: 'Files and backups' },
  { id: 'logins', label: 'Logins and captchas' },
];

export const kindLabel = (kind: string): string => KINDS.find((k) => k.kind === kind)?.label ?? kind;

export const persistedKinds = (category: Category): string[] =>
  KINDS.filter((k) => !k.transient && k.category === category).map((k) => k.kind);
