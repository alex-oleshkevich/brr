export type NavSection = 'Work' | 'Library';

export type NavItem = { href: string; label: string; section: NavSection };

export const nav: NavItem[] = [
  { href: '/', label: 'Overview', section: 'Work' },
  { href: '/browsers', label: 'Browsers', section: 'Work' },
  { href: '/history', label: 'History', section: 'Work' },
  { href: '/profiles', label: 'Profiles', section: 'Library' },
  { href: '/templates', label: 'Templates', section: 'Library' },
  { href: '/extensions', label: 'Extensions', section: 'Library' },
  { href: '/logins', label: 'Logins', section: 'Library' },
  { href: '/versions', label: 'Browser versions', section: 'Library' },
];
