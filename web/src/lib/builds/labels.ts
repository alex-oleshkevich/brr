import type { components } from '$lib/api/schema';

export type BuildSource = components['schemas']['BuildSource'];
export type Engine = components['schemas']['Engine'];

export const SOURCE_NAMES: Record<BuildSource, string> = {
  cft: 'Chrome for Testing',
  headless_shell: 'Chrome Headless Shell',
  ungoogled: 'ungoogled-chromium',
  fingerprint_chromium: 'Fingerprint Chromium',
  brave: 'Brave',
  edge: 'Microsoft Edge',
  system: 'System',
  camoufox: 'Camoufox',
};

export const INSTALLABLE_SOURCES: BuildSource[] = ['cft', 'headless_shell', 'ungoogled', 'fingerprint_chromium', 'brave', 'edge', 'camoufox'];

export const SOURCE_ENGINE: Record<BuildSource, Engine> = {
  cft: 'chromium',
  headless_shell: 'chromium',
  ungoogled: 'chromium',
  fingerprint_chromium: 'chromium',
  brave: 'chromium',
  edge: 'chromium',
  system: 'chromium',
  camoufox: 'camoufox',
};

const MULTI_CHANNEL: BuildSource[] = ['cft', 'headless_shell'];

export function channelsFor(source: BuildSource): { value: string; label: string }[] {
  if (source === 'camoufox') return [{ value: 'latest-compatible', label: 'Latest compatible' }];
  if (MULTI_CHANNEL.includes(source)) {
    return ['stable', 'beta', 'dev', 'canary'].map((name) => ({ value: name, label: name[0].toUpperCase() + name.slice(1) }));
  }
  return [{ value: 'stable', label: 'Stable' }];
}

// A system browser keeps its own name (Google Chrome, Brave): a build record carries it as `name`, an Instance's build ref as `label`.
export function buildLabel(build: { source: BuildSource; version: string; name?: string; label?: string }): string {
  if (build.source === 'system') {
    if (build.label) return build.label;
    if (build.name) return `${build.name} ${build.version}`;
  }
  return `${SOURCE_NAMES[build.source]} ${build.version}`;
}
