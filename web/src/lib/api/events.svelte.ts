import type { components } from './schema';
import { noteUiClient, session, startSession } from './session';

export type BrrEvent = components['schemas']['Event'];
export type StreamStatus = 'connecting' | 'open' | 'reconnecting';

const HEARTBEAT_TIMEOUT_MS = 45_000;
const BACKOFF_MS = [1000, 2000, 4000, 8000, 10_000];

type Listener = { kinds: readonly string[] | '*'; fn: (e: BrrEvent) => void };

function matches(kinds: readonly string[] | '*', kind: string): boolean {
  if (kinds === '*') return true;
  return kinds.some((k) => (k.endsWith('.*') ? kind.startsWith(k.slice(0, -1)) : k === kind));
}

class EventStream {
  status = $state<StreamStatus>('connecting');
  lastSeq: number | null = null;

  private listeners = new Set<Listener>();
  private resyncs = new Set<() => void>();
  private attempt = 0;
  private started = false;
  private opened = false;
  private abort: AbortController | null = null;

  connect(): void {
    if (this.started) return;
    this.started = true;
    void this.run();
  }

  on(kinds: readonly string[] | '*', fn: (e: BrrEvent) => void): () => void {
    const listener = { kinds, fn };
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  onResync(fn: () => void): () => void {
    this.resyncs.add(fn);
    return () => this.resyncs.delete(fn);
  }

  private async run(): Promise<void> {
    for (;;) {
      const immediate = await this.readOnce();
      this.status = 'reconnecting';
      if (immediate) continue;

      const delay = BACKOFF_MS[Math.min(this.attempt, BACKOFF_MS.length - 1)];
      this.attempt++;
      await new Promise((resolve) => setTimeout(resolve, delay));
    }
  }

  // Returns true when the caller should reconnect without waiting.
  private async readOnce(): Promise<boolean> {
    const abort = new AbortController();
    this.abort = abort;

    let watchdog: ReturnType<typeof setTimeout>;
    const feed = () => {
      clearTimeout(watchdog);
      watchdog = setTimeout(() => abort.abort(), HEARTBEAT_TIMEOUT_MS);
    };
    feed();

    try {
      const response = await fetch('/v1/events/stream', {
        headers: {
          Accept: 'text/event-stream',
          'Brr-Client': session.clientId ?? '',
          ...(this.lastSeq !== null && { 'Last-Event-ID': String(this.lastSeq) }),
        },
        signal: abort.signal,
      });

      if (response.status === 404) {
        const body = await response.json().catch(() => null);
        if (body?.code === 'NotFound') {
          await startSession();
          return true;
        }
      }
      if (!response.ok || !response.body) return false;

      this.status = 'open';
      this.attempt = 0;
      // Everything mounted loaded after the page did; only a reconnect can have missed events.
      if (this.opened) this.resyncs.forEach((fn) => fn());
      this.opened = true;

      return await this.read(response.body, feed, abort);
    } catch {
      return false;
    } finally {
      clearTimeout(watchdog!);
    }
  }

  private async read(body: ReadableStream<Uint8Array>, feed: () => void, abort: AbortController): Promise<boolean> {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let pending = '';
    let id: string | null = null;
    let name = '';
    let data: string[] = [];

    const dispatch = (): boolean => {
      const lagged = name === 'lagged';
      if (id !== null && /^\d+$/.test(id)) this.lastSeq = Number(id);
      if (!lagged && data.length > 0) this.emit(data.join('\n'));
      id = null;
      name = '';
      data = [];
      return lagged;
    };

    for (;;) {
      const { done, value } = await reader.read();
      if (done) return false;
      feed();

      pending += decoder.decode(value, { stream: true });
      const lines = pending.split('\n');
      pending = lines.pop() ?? '';

      for (const raw of lines) {
        const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw;
        if (line === '') {
          if (dispatch()) {
            abort.abort();
            return true;
          }
          continue;
        }
        if (line.startsWith(':')) continue;

        const colon = line.indexOf(':');
        const field = colon === -1 ? line : line.slice(0, colon);
        let content = colon === -1 ? '' : line.slice(colon + 1);
        if (content.startsWith(' ')) content = content.slice(1);

        if (field === 'id') id = content;
        else if (field === 'event') name = content;
        else if (field === 'data') data.push(content);
      }
    }
  }

  private emit(text: string): void {
    let event: BrrEvent;
    try {
      event = JSON.parse(text);
    } catch {
      return;
    }
    if (typeof event?.kind !== 'string') return;

    noteUiClient(event);
    this.listeners.forEach((listener) => {
      if (matches(listener.kinds, event.kind)) listener.fn(event);
    });
  }
}

export const events = new EventStream();
