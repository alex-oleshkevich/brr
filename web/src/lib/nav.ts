export type NavSection = 'Run' | 'Set up' | 'System';

// `also` lists extra path prefixes that keep the item highlighted (Templates live inside Profiles).
export type NavItem = { href: string; label: string; section: NavSection; also?: string[] };

export const nav: NavItem[] = [
  { href: '/', label: 'Dashboard', section: 'Run' },
  { href: '/instances', label: 'Browsers', section: 'Run' },
  { href: '/events', label: 'Activity', section: 'Run' },
  { href: '/files', label: 'Files', section: 'Run' },
  { href: '/connect', label: 'Connect an agent', section: 'Run' },
  { href: '/profiles', label: 'Profiles', section: 'Set up', also: ['/templates'] },
  { href: '/proxies', label: 'Proxies', section: 'Set up' },
  { href: '/credentials', label: 'Logins', section: 'Set up' },
  { href: '/extensions', label: 'Extensions', section: 'Set up' },
  { href: '/builds', label: 'Browser versions', section: 'System' },
  { href: '/captcha', label: 'Captcha', section: 'System' },
  { href: '/limits', label: 'Capacity', section: 'System' },
  { href: '/storage', label: 'Storage', section: 'System' },
  { href: '/settings', label: 'Settings', section: 'System' },
];
