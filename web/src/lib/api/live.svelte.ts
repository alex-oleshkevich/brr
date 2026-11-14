import { ApiError } from './client';
import { events, type BrrEvent } from './events.svelte';

export interface LiveQuery<T> {
  readonly data: T | undefined;
  readonly error: ApiError | null;
  readonly loading: boolean;
  reload(): void;
}

// Event-driven refetches are coalesced: the first event starts the timer, later ones ride along.
const COALESCE_MS = 500;

export function liveQuery<T>(
  load: (fresh: boolean) => Promise<T>,
  opts: { kinds: readonly string[]; match?: (e: BrrEvent) => boolean; refreshMs?: number },
): LiveQuery<T> {
  let data = $state<T>();
  let error = $state<ApiError | null>(null);
  let loading = $state(true);
  let reload = () => {};

  $effect(() => {
    // Only changed inputs re-run this effect; events refresh through run() and keep the last data on screen.
    data = undefined;
    error = null;
    loading = true;

    let running = false;
    let again = false;
    let debounce: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    let fresh = true;

    async function run() {
      if (running) {
        again = true;
        return;
      }
      running = true;
      try {
        // load() reads its reactive inputs synchronously before its first await, so the effect tracks them.
        const first = fresh;
        fresh = false;
        const next = await load(first);
        if (disposed) return;
        data = next;
        error = null;
      } catch (e) {
        if (!disposed && data === undefined) {
          error = e instanceof ApiError ? e : new ApiError(0, 'Unreachable', String(e), {});
        }
      } finally {
        running = false;
        if (!disposed) loading = false;
      }
      if (again && !disposed) {
        again = false;
        void run();
      }
    }

    reload = () => void run();
    const schedule = () => {
      if (debounce) return;
      debounce = setTimeout(() => {
        debounce = undefined;
        void run();
      }, COALESCE_MS);
    };

    void run();

    const offEvents = events.on(opts.kinds, (e) => {
      if (!opts.match || opts.match(e)) schedule();
    });
    const offResync = events.onResync(schedule);
    const timer = opts.refreshMs
      ? setInterval(() => {
          // The event stream covers a connected page; the poll only stands in while it is down.
          if (document.visibilityState === 'visible' && events.status !== 'open') void run();
        }, opts.refreshMs)
      : undefined;

    return () => {
      disposed = true;
      clearTimeout(debounce);
      clearInterval(timer);
      offEvents();
      offResync();
    };
  });

  return {
    get data() {
      return data;
    },
    get error() {
      return error;
    },
    get loading() {
      return loading;
    },
    reload: () => reload(),
  };
}
