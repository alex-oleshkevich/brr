import type { components } from '$lib/api/schema';

export type Proxy = components['schemas']['Proxy'];

// The same host:port can exist with different logins, so labels for menus and pickers include the login.
export const proxyLabel = (p: Pick<Proxy, 'scheme' | 'host' | 'port' | 'username'>) =>
  `${p.scheme}://${p.username ? `${p.username}@` : ''}${p.host}:${p.port}`;

export const proxyUrl = (p: Pick<Proxy, 'scheme' | 'host' | 'port'>) => `${p.scheme}://${p.host}:${p.port}`;

const SCHEMES = ['http', 'https', 'socks5'];
const HOST = /^(?:[A-Za-z0-9.-]{1,253}|\[[0-9A-Fa-f:.]+\])$/;

// Mirrors the daemon's import parser; returns why a line is rejected, or null when it is valid.
export function importLineError(line: string): string | null {
  let host: string;
  let port: string;
  let user = '';
  let password = '';

  const login = (value: string) => {
    const at = value.indexOf(':');
    [user, password] = at < 0 ? [value, ''] : [value.slice(0, at), value.slice(at + 1)];
  };

  const sep = line.indexOf('://');
  if (sep >= 0 || line.includes('@')) {
    let rest = line;
    if (sep >= 0) {
      if (!SCHEMES.includes(line.slice(0, sep))) return 'scheme must be http, https or socks5';
      rest = line.slice(sep + 3);
    }
    const at = rest.lastIndexOf('@');
    if (at >= 0) login(rest.slice(0, at));
    const address = rest.slice(at + 1);
    const colon = address.lastIndexOf(':');
    if (colon < 0) return 'missing :port';
    host = address.slice(0, colon);
    port = address.slice(colon + 1);
  } else {
    const parts = line.split(':');
    if (parts.length < 2) return 'missing :port';
    if (parts.length === 3) return 'a login needs both user and password';
    [host, port] = parts;
    if (parts.length > 3) {
      user = parts[2];
      password = parts.slice(3).join(':');
    }
  }

  if (!HOST.test(host)) return 'invalid host';
  if (!/^\d{1,5}$/.test(port) || Number(port) < 1 || Number(port) > 65535) return 'port must be 1 to 65535';
  if (user.length > 255 || user.includes(':')) return 'username too long';
  if (password.length > 1024) return 'password too long';
  return null;
}

export type ParsedProxyUrl = { scheme: string; host: string; port: number; username: string; password: string };

// Reads a pasted `scheme://user:pass@host:port`; null when the text is not a full proxy URL.
export function parseProxyUrl(text: string): ParsedProxyUrl | null {
  const match = /^(https?|socks5):\/\/(?:([^:@/]*)(?::([^@/]*))?@)?([^:/@]+|\[[0-9A-Fa-f:.]+\]):(\d{1,5})\/?$/.exec(text.trim());
  if (!match) return null;
  const [, scheme, username = '', password = '', host, port] = match;
  return { scheme, host, port: Number(port), username, password };
}
