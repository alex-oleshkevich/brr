import type { ApiError } from './client';
import { liveQuery } from './live.svelte';
import type { BrrEvent } from './events.svelte';
import { toast } from '$lib/ui/toast.svelte';

export interface Page<T> {
  items: T[];
  next: string | null | undefined;
}

export interface PagedQuery<T> {
  readonly items: T[] | undefined;
  readonly error: ApiError | null;
  readonly loading: boolean;
  readonly hasMore: boolean;
  readonly capped: boolean;
  readonly max: number;
  readonly loadingMore: boolean;
  reload(): void;
  loadMore(): Promise<void>;
  prepend(item: T): void;
}

const MAX_ITEMS = 1000;

// For lists a screen needs whole (a lookup, a dropdown) rather than page by page.
export async function fetchAll<T>(fetchPage: (cursor?: string) => Promise<Page<T>>): Promise<T[]> {
  const all: T[] = [];
  let cursor: string | undefined;
  do {
    const page = await fetchPage(cursor);
    all.push(...page.items);
    cursor = page.next ?? undefined;
  } while (cursor);

  return all;
}

interface Options<T> {
  kinds: readonly string[];
  key: (item: T) => string | number;
  moreError: string;
  match?: (e: BrrEvent) => boolean;
  refreshMs?: number;
  max?: number;
}

// One cursor-paginated list. An event or timer refresh refetches the whole loaded window, a changed input starts over,
// and a response that belongs to an older input is dropped. At `max` rows the list stops offering more, so it never has a gap.
export function pagedQuery<T>(fetchPage: (cursor?: string) => Promise<Page<T>>, opts: Options<T>): PagedQuery<T> {
  const max = opts.max ?? MAX_ITEMS;
  let pagesLoaded = 1;
  let epoch = 0;

  let live = $state.raw<T[]>([]);
  let tail = $state.raw<{ anchor: unknown; items: T[]; next: string | null | undefined }>({ anchor: null, items: [], next: undefined });
  let loadingMore = $state(false);

  const base = liveQuery(
    async (fresh) => {
      if (fresh) {
        pagesLoaded = 1;
        epoch++;
        live = [];
      }

      // The first call runs before any await, so liveQuery tracks the inputs fetchPage reads.
      let page = await fetchPage();
      const items = [...page.items];
      for (let n = 1; n < pagesLoaded && page.next; n++) {
        page = await fetchPage(page.next);
        items.push(...page.items);
      }

      return { items, next: page.next ?? null };
    },
    opts,
  );

  const tailValid = $derived(tail.anchor === base.data);
  const tailItems = $derived(tailValid ? tail.items : []);
  const next = $derived(tailValid ? (tail.next ?? null) : (base.data?.next ?? null));
  const merged = $derived.by(() => {
    if (!base.data) return undefined;
    const seen = new Set<string | number>();

    return [...live, ...base.data.items, ...tailItems].filter((item) => {
      const key = opts.key(item);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  });

  async function loadMore() {
    const cursor = next;
    if (!cursor || loadingMore) return;
    const started = epoch;
    loadingMore = true;
    try {
      const page = await fetchPage(cursor);
      if (started !== epoch) return;
      tail = { anchor: base.data, items: [...tailItems, ...page.items], next: page.next ?? null };
      pagesLoaded++;
    } catch (e) {
      if (started === epoch) toast.apiError(e, opts.moreError);
    } finally {
      loadingMore = false;
    }
  }

  return {
    get items() {
      return merged?.slice(0, max);
    },
    get error() {
      return base.error;
    },
    get loading() {
      return base.loading;
    },
    get hasMore() {
      return next !== null && (merged?.length ?? 0) < max;
    },
    get capped() {
      return (merged?.length ?? 0) >= max;
    },
    max,
    get loadingMore() {
      return loadingMore;
    },
    reload: () => base.reload(),
    loadMore,
    prepend: (item) => (live = [item, ...live].slice(0, max)),
  };
}
