import type { components } from '$lib/api/schema';
import { SOURCE_NAMES, type BuildSource } from '$lib/builds/labels';

type BuildView = components['schemas']['BuildView'];
type BuildChoice = components['schemas']['ProfileSettings']['build'];

export type Resolved =
  | { kind: 'build'; build: BuildView; fallback: boolean }
  | { kind: 'missing'; source: BuildSource; channel: string }
  | { kind: 'unknown' };

const numericParts = (version: string) => version.split(/[.-]/).map((part) => Number.parseInt(part, 10) || 0);

function compareVersions(a: string, b: string): number {
  const left = numericParts(a);
  const right = numericParts(b);
  for (let i = 0; i < Math.max(left.length, right.length); i++) {
    const diff = (left[i] ?? 0) - (right[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

export const parseChannel = (channel: string): { source: BuildSource; name: string } => {
  const [source, ...rest] = channel.split(':');
  return { source: source as BuildSource, name: rest.join(':') };
};

// Mirrors the daemon's pick_build: the newest installed build of the channel; for Chrome for Testing,
// the detected system browser when none is installed.
export function resolveBuild(choice: BuildChoice, builds: BuildView[]): Resolved {
  if ('build_id' in choice) {
    const pinned = builds.find((b) => b.id === choice.build_id && (b.state === 'installed' || b.state === 'registered'));
    return pinned ? { kind: 'build', build: pinned, fallback: false } : { kind: 'unknown' };
  }
  const { source, name } = parseChannel(choice.channel);
  const current = builds
    .filter((b) => b.source === source && b.state === 'installed' && b.channel === name)
    .toSorted((a, b) => compareVersions(b.version, a.version))[0];
  if (current) return { kind: 'build', build: current, fallback: false };
  const system = source === 'cft' ? builds.find((b) => b.source === 'system' && b.state === 'registered') : undefined;
  return system ? { kind: 'build', build: system, fallback: true } : { kind: 'missing', source, channel: name };
}

export const channelTitle = (name: string) => (name === 'latest-compatible' ? 'latest compatible' : name[0].toUpperCase() + name.slice(1));

// "Google Chrome 157.0.8103.78" -> "System Chrome 157"; everything else keeps its source name and full version.
function buildTitle(build: BuildView): string {
  if (build.source === 'system') return `System ${build.name.replace(/^Google /, '')} ${build.version.split('.')[0]}`;
  return `${SOURCE_NAMES[build.source]} ${build.version}`;
}

export function resolvedLabel(choice: BuildChoice, resolved: Resolved): string {
  if (resolved.kind === 'build') {
    const channel = 'channel' in choice ? ` (${channelTitle(parseChannel(choice.channel).name)})` : '';
    return `${buildTitle(resolved.build)}${channel}`;
  }
  return '';
}

export function missingLabel(resolved: Extract<Resolved, { kind: 'missing' }>): string {
  return `No ${SOURCE_NAMES[resolved.source]} ${channelTitle(resolved.channel)} build installed`;
}

// Option label for the channel picker: "Chrome for Testing · Stable (installed)".
export function channelOptionLabel(source: BuildSource, name: string, builds: BuildView[]): string {
  const installed = builds.some((b) => b.source === source && b.state === 'installed' && b.channel === name);
  return `${SOURCE_NAMES[source]} · ${channelTitle(name)} (${installed ? 'installed' : 'not installed'})`;
}
