import type { components } from '$lib/api/schema';

export type Template = components['schemas']['Template'];

// Plain titles for the built-in templates; their slugs are internal names.
const BUILTIN_TITLES: Record<string, string> = {
  'chromium-headless': 'Chrome — headless (recommended for agents)',
  'chromium-visible': 'Chrome — visible window on this computer',
  'camoufox-stealth': 'Firefox stealth (Camoufox) — off-screen display',
  'camoufox-headless': 'Firefox stealth (Camoufox) — headless',
};

export const RECOMMENDED_TEMPLATE = 'chromium-headless';

export const templateTitle = (t: Template) => (t.builtin ? (BUILTIN_TITLES[t.name] ?? t.name) : t.name);
export const templatePurpose = (t: Template) => t.description?.trim() || '';
