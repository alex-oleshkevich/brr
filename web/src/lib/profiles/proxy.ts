import type { components } from '$lib/api/schema';

export type Proxy = components['schemas']['Proxy'];

export const proxyName = (p: Proxy) => `${p.scheme}://${p.host}:${p.port}`;

export const isUnhealthy = (p: Proxy) => p.healthy === false;

// Option label for pickers: "http://host:3128 — unhealthy", "— healthy" or no marker when it was never checked.
export function proxyOptionLabel(p: Proxy): string {
  if (p.healthy === false) return `${proxyName(p)} — unhealthy`;
  if (p.healthy === true) return `${proxyName(p)} — healthy`;
  return proxyName(p);
}

export const unhealthyReason = (p: Proxy) => p.last_error?.trim() || 'it failed its last health check';
