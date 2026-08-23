/**
 * Display paging over the accumulating row buffer.
 *
 *   App.svelte owns the buffer:  tab.rows grows by CHUNK_SIZE per fetch
 *          │
 *          │ rows (accumulated) · fetchedCount · isEnd
 *          ▼
 *   createPagedStream slices it:  pageRows = rows.slice(p*size, (p+1)*size)
 *          │
 *          └── asks for more via onNeedMore when the user walks past fetchedCount
 *
 * Every clamp here exists because of a specific race. Read the comments before
 * changing any of them.
 *
 * MUST be called synchronously during component init — it opens $effect scopes.
 */
export interface PagedStreamConfig {
  readonly rows: unknown[][];
  readonly fetchedCount: number;
  readonly isEnd: boolean;
  readonly totalCount: number | null;
  readonly pageSize: number;
  /** Changing this resets paging — the grid is reused across tabs. */
  readonly tabId: unknown;
  onNeedMore?: () => Promise<void> | void;
  /** Called when paging resets, so the caller can scroll the viewport home. */
  onScrollReset?: () => void;
}

export function createPagedStream(config: PagedStreamConfig) {
  let page = $state(0);
  let isFetchingMore = $state(false);

  const maxPage = $derived(
    Math.max(0, Math.ceil(config.fetchedCount / config.pageSize) - 1),
  );

  const pageRows = $derived(
    config.rows.slice(page * config.pageSize, (page + 1) * config.pageSize),
  );

  // "Next" is enabled unless we've reached the end AND the next page isn't cached.
  const canGoNext = $derived(
    !config.isEnd || (page + 1) * config.pageSize < config.fetchedCount,
  );

  /** The whole result is in hand and fits one page — nothing left to page. */
  const fitsOnePage = $derived(
    config.isEnd && config.fetchedCount <= config.pageSize,
  );

  const firstRowNumber = $derived(
    Math.min(page * config.pageSize + 1, config.fetchedCount),
  );
  const lastRowNumber = $derived(
    Math.min((page + 1) * config.pageSize, config.fetchedCount),
  );

  function reset() {
    page = 0;
    isFetchingMore = false;
    config.onScrollReset?.();
  }

  // Reset on tab switch. Svelte reuses the component instance for the same
  // {#if} branch, so internal state persists across tab switches without this.
  $effect(() => {
    void config.tabId;
    reset();
  });

  // Reset to page 0 when a fresh fetch arrives (fetchedCount drops to <= one
  // page after a sort/filter change or a re-execute in the same tab).
  $effect(() => {
    void config.fetchedCount;
    if (config.fetchedCount > 0 && config.fetchedCount <= config.pageSize) {
      page = 0;
      config.onScrollReset?.();
    }
  });

  // Clamp so the page never points past the fetched data. Covers goNext
  // advancing before fetchedCount catches up, and tab state resets.
  $effect(() => {
    void maxPage;
    if (page > maxPage) page = maxPage;
  });

  async function goNext() {
    // Local guard against racing ahead of an in-flight fetch on rapid clicks.
    if (isFetchingMore) return;
    const nextPage = page + 1;
    if (nextPage * config.pageSize >= config.fetchedCount) {
      isFetchingMore = true;
      try {
        await config.onNeedMore?.();
      } finally {
        isFetchingMore = false;
      }
    }
    // Only advance if the next page actually has data now — either it was
    // cached, or the fetch filled it. isEnd means "no more rows" and must
    // NEVER let us advance past the last valid page.
    if (config.fetchedCount > nextPage * config.pageSize) {
      page = nextPage;
    }
  }

  function goPrev() {
    page = Math.max(0, page - 1);
  }

  return {
    get page() {
      return page;
    },
    get pageRows() {
      return pageRows;
    },
    get maxPage() {
      return maxPage;
    },
    get canGoNext() {
      return canGoNext;
    },
    get fitsOnePage() {
      return fitsOnePage;
    },
    get isFetchingMore() {
      return isFetchingMore;
    },
    get firstRowNumber() {
      return firstRowNumber;
    },
    get lastRowNumber() {
      return lastRowNumber;
    },
    goNext,
    goPrev,
    reset,
  };
}
