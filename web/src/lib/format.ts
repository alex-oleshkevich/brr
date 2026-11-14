const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

export function bytes(n: number | null): string {
  if (n === null) return '—';
  let value = n;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit++;
  }
  const text = unit > 0 && value < 10 ? value.toFixed(1) : Math.round(value).toString();
  return `${text} ${UNITS[unit]}`;
}

export function duration(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);
  if (days > 0) return `${days}d ${hours % 24}h`;
  if (hours > 0) return `${hours}h ${minutes % 60}m`;
  if (minutes > 0) return `${minutes}m`;
  return `${Math.floor(ms / 1000)}s`;
}

export function relative(ts: number, now: number): string {
  const diff = now - ts;
  // The clock ticks every 15 s, so a fresh server timestamp can sit slightly ahead of it.
  if (diff < 10_000 && diff > -60_000) return 'just now';
  return diff > 0 ? `${duration(diff)} ago` : `in ${duration(-diff)}`;
}

const absoluteFormat = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'medium' });
const countFormat = new Intl.NumberFormat();
const usdFormat = new Intl.NumberFormat(undefined, { style: 'currency', currency: 'USD', maximumFractionDigits: 6 });

export function absolute(ts: number): string {
  return absoluteFormat.format(ts);
}

export function count(n: number): string {
  return countFormat.format(n);
}

export function usd(micros: number): string {
  return usdFormat.format(micros / 1_000_000);
}

export function shortId(id: string): string {
  return '…' + id.slice(-6);
}

// A secret's hint arrives as '…' plus its last characters, or only '…' when it is too short to show any.
export function setHint(hint: string | null | undefined): string {
  return !hint || hint === '…' ? 'Set' : `Set · ends in ${hint.slice(1)}`;
}
