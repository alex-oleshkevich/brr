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

// Friendly label for a stored template name (a built-in's slug becomes its plain title).
export const templateNameLabel = (name: string | null | undefined): string =>
  name ? (BUILTIN_TITLES[name] ?? name) : '—';

// Short friendly label for tables: "Chrome — headless (recommended for agents)" -> "Chrome — headless".
export const templateShortNameLabel = (name: string | null | undefined): string => templateNameLabel(name).split(' (')[0];

// A valid template name suggested from a friendly title: "Chrome — headless (…)" -> "chrome-headless".
export const templateSlug = (title: string): string =>
  title
    .split('(')[0]
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64) || 'template';

export const NAME_HINT = 'Use 1 to 64 characters: letters, numbers, dot, underscore or hyphen.';

// First free name for a copy: "<base>-copy", then "<base>-copy-2", "-copy-3", …
export function uniqueCopyName(base: string, taken: Set<string>): string {
  if (!taken.has(`${base}-copy`)) return `${base}-copy`;
  let n = 2;
  while (taken.has(`${base}-copy-${n}`)) n += 1;
  return `${base}-copy-${n}`;
}
