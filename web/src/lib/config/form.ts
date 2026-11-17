import type { components } from '$lib/api/schema';
import { SOURCE_ENGINE, type BuildSource } from '$lib/builds/labels';

export type Settings = components['schemas']['ProfileSettings'];
export type GlobalSettings = components['schemas']['Settings'];
export type BuildView = components['schemas']['BuildView'];
export type Proxy = components['schemas']['Proxy'];
export type Pool = components['schemas']['Pool'];
export type ExtensionView = components['schemas']['ExtensionView'];
export type ProfileExtension = components['schemas']['ProfileExtension'];

export type SectionProps = {
  settings: Settings;
  readonly?: boolean;
  errors: Record<string, string>;
  builds: BuildView[];
  proxies: Proxy[];
  pools: Pool[];
  library: ExtensionView[];
  defaults?: GlobalSettings;
  profileExtensions: ProfileExtension[];
  onExtensions: (next: ProfileExtension[]) => void;
  extUnsupported: boolean;
  isTemplate: boolean;
  pinned: boolean;
  profileId?: string | null;
  profileKind?: string | null;
  reportInvalid: (id: string, invalid: boolean) => void;
  onPinned: (value: boolean) => void;
  quotaGb: number | null;
  onQuota: (value: number | null) => void;
};

export function sourceOf(settings: Settings, builds: BuildView[]): BuildSource | null {
  const build = settings.build;
  if ('channel' in build) return build.channel.split(':')[0] as BuildSource;
  return builds.find((b) => b.id === build.build_id)?.source ?? null;
}

export function engineOf(settings: Settings, builds: BuildView[]): 'chromium' | 'camoufox' {
  const choice = settings.build;
  if ('build_id' in choice) {
    const build = builds.find((b) => b.id === choice.build_id);
    if (build) return build.engine;
  }
  const source = sourceOf(settings, builds);
  return source ? SOURCE_ENGINE[source] : 'chromium';
}

// First error recorded under any of the given field paths.
export function fieldError(errors: Record<string, string>, ...keys: string[]): string | null {
  for (const key of keys) if (errors[key]) return errors[key];
  return null;
}

// JSON with recursively sorted keys, so field order from the server never reads as a change.
export function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    return `{${Object.entries(value as Record<string, unknown>)
      .sort(([a], [b]) => (a < b ? -1 : 1))
      .map(([k, v]) => `${JSON.stringify(k)}:${canonical(v)}`)
      .join(',')}}`;
  }
  return JSON.stringify(value) ?? 'null';
}

// Width/height style fields are saved together or not at all: one filled and one empty is an error on the empty one.
export function halfPair(a: number | null, b: number | null): { first: boolean; second: boolean } {
  return { first: a === null && b !== null, second: a !== null && b === null };
}
