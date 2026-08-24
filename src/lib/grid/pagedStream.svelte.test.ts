import { describe, it, expect, vi } from 'vitest';
import { flushSync } from 'svelte';
import { createPagedStream } from './pagedStream.svelte.ts';

/**
 * Mutable backing state plus a stream reading it through getters, mirroring how
 * ResultsGrid feeds the controller. `flush` lets a test await the microtask the
 * clamping effects run in.
 *
 * $effect requires an owner, so every case runs inside $effect.root and disposes
 * at the end — the same constraint the real component satisfies by construction.
 * `state` is a $state proxy because ResultsGrid feeds the controller from
 * $state-backed props: the getters must read tracked signals, or deriveds like
 * maxPage compute once at mount and never see later fetches. flushSync()
 * settles the mount-time reset/clamp effects before any test interacts with
 * the stream — in a real component those effects always settle during init,
 * before the first user interaction can reach goNext/goPrev.
 *
 * This file is named *.svelte.test.ts because it uses runes ($effect.root)
 * directly, and only .svelte-infixed files get the Svelte compiler here.
 */
function harness(init: {
  rows?: unknown[][];
  fetchedCount?: number;
  isEnd?: boolean;
  pageSize?: number;
  onNeedMore?: () => Promise<void> | void;
}) {
  const state = $state({
    rows: init.rows ?? [],
    fetchedCount: init.fetchedCount ?? 0,
    isEnd: init.isEnd ?? false,
    totalCount: null as number | null,
    pageSize: init.pageSize ?? 200,
    tabId: 't1',
  });
  let stream!: ReturnType<typeof createPagedStream>;
  const dispose = $effect.root(() => {
    stream = createPagedStream({
      get rows() { return state.rows; },
      get fetchedCount() { return state.fetchedCount; },
      get isEnd() { return state.isEnd; },
      get totalCount() { return state.totalCount; },
      get pageSize() { return state.pageSize; },
      get tabId() { return state.tabId; },
      onNeedMore: init.onNeedMore,
    });
  });
  flushSync();
  const flush = () => new Promise((r) => setTimeout(r, 0));
  return { state, stream: () => stream, flush, dispose };
}

/** N rows of one column, values 0..N-1, so a test can identify a page by value. */
function rowsOf(n: number): unknown[][] {
  return Array.from({ length: n }, (_, i) => [i]);
}

describe('page slicing', () => {
  it('slices the current page out of the accumulated buffer', () => {
    const h = harness({ rows: rowsOf(500), fetchedCount: 500, pageSize: 200 });
    expect(h.stream().pageRows).toHaveLength(200);
    expect(h.stream().pageRows[0]).toEqual([0]);
    h.dispose();
  });

  it('gives the last page only its remaining rows', async () => {
    const h = harness({ rows: rowsOf(450), fetchedCount: 450, isEnd: true, pageSize: 200 });
    await h.stream().goNext();
    await h.stream().goNext();
    expect(h.stream().page).toBe(2);
    expect(h.stream().pageRows).toHaveLength(50);
    h.dispose();
  });
});

describe('canGoNext', () => {
  it('is true while more rows may exist on the server', () => {
    const h = harness({ rows: rowsOf(200), fetchedCount: 200, isEnd: false });
    expect(h.stream().canGoNext).toBe(true);
    h.dispose();
  });

  it('is false at the end when the next page is not cached', () => {
    const h = harness({ rows: rowsOf(200), fetchedCount: 200, isEnd: true });
    expect(h.stream().canGoNext).toBe(false);
    h.dispose();
  });

  it('is true at the end when the next page IS cached', () => {
    const h = harness({ rows: rowsOf(450), fetchedCount: 450, isEnd: true });
    expect(h.stream().canGoNext).toBe(true);
    h.dispose();
  });
});

describe('goNext', () => {
  it('fetches when the next page is past what is fetched', async () => {
    const onNeedMore = vi.fn(async () => {});
    const h = harness({ rows: rowsOf(200), fetchedCount: 200, onNeedMore });
    await h.stream().goNext();
    expect(onNeedMore).toHaveBeenCalledTimes(1);
    h.dispose();
  });

  it('does not fetch when the next page is already cached', async () => {
    const onNeedMore = vi.fn(async () => {});
    const h = harness({ rows: rowsOf(400), fetchedCount: 400, onNeedMore });
    await h.stream().goNext();
    expect(onNeedMore).not.toHaveBeenCalled();
    expect(h.stream().page).toBe(1);
    h.dispose();
  });

  it('refuses to advance when the fetch brought nothing back', async () => {
    // isEnd means no more rows. Advancing here would show an empty page.
    const h = harness({
      rows: rowsOf(200),
      fetchedCount: 200,
      isEnd: true,
      onNeedMore: async () => {},
    });
    await h.stream().goNext();
    expect(h.stream().page).toBe(0);
    h.dispose();
  });

  it('advances once when the fetch filled the page', async () => {
    const h = harness({
      rows: rowsOf(200),
      fetchedCount: 200,
      onNeedMore: async () => {
        h.state.rows = rowsOf(400);
        h.state.fetchedCount = 400;
      },
    });
    await h.stream().goNext();
    expect(h.stream().page).toBe(1);
    h.dispose();
  });

  it('ignores a second click while a fetch is in flight', async () => {
    let calls = 0;
    let release!: () => void;
    const gate = new Promise<void>((r) => { release = r; });
    const h = harness({
      rows: rowsOf(200),
      fetchedCount: 200,
      onNeedMore: async () => { calls += 1; await gate; },
    });
    const first = h.stream().goNext();
    const second = h.stream().goNext();
    release();
    await Promise.all([first, second]);
    // The guard is what stops rapid Next clicks racing ahead of the data.
    expect(calls).toBe(1);
    h.dispose();
  });
});

describe('goPrev', () => {
  it('steps back one page', async () => {
    const h = harness({ rows: rowsOf(400), fetchedCount: 400 });
    await h.stream().goNext();
    h.stream().goPrev();
    expect(h.stream().page).toBe(0);
    h.dispose();
  });

  it('never goes below zero', () => {
    const h = harness({ rows: rowsOf(200), fetchedCount: 200 });
    h.stream().goPrev();
    expect(h.stream().page).toBe(0);
    h.dispose();
  });
});

describe('clamping', () => {
  it('pulls the page back when fetchedCount shrinks under it', async () => {
    const h = harness({ rows: rowsOf(600), fetchedCount: 600 });
    await h.stream().goNext();
    await h.stream().goNext();
    expect(h.stream().page).toBe(2);

    // A refetch after a filter change drops the buffer to one page.
    h.state.rows = rowsOf(200);
    h.state.fetchedCount = 200;
    await h.flush();

    expect(h.stream().page).toBe(0);
    h.dispose();
  });

  it('returns to page 0 when a fresh fetch arrives', async () => {
    const h = harness({ rows: rowsOf(400), fetchedCount: 400 });
    await h.stream().goNext();
    h.state.rows = rowsOf(150);
    h.state.fetchedCount = 150;
    await h.flush();
    expect(h.stream().page).toBe(0);
    h.dispose();
  });

  it('resets to page 0 on a tab switch', async () => {
    const h = harness({ rows: rowsOf(400), fetchedCount: 400 });
    await h.stream().goNext();
    h.state.tabId = 't2';
    await h.flush();
    expect(h.stream().page).toBe(0);
    h.dispose();
  });
});

describe('fitsOnePage', () => {
  it('is true only when the whole result is in hand and fits', () => {
    const h = harness({ rows: rowsOf(50), fetchedCount: 50, isEnd: true });
    expect(h.stream().fitsOnePage).toBe(true);
    h.dispose();
  });

  it('is false while more rows may arrive', () => {
    const h = harness({ rows: rowsOf(50), fetchedCount: 50, isEnd: false });
    expect(h.stream().fitsOnePage).toBe(false);
    h.dispose();
  });
});

describe('row numbers for the footer', () => {
  it('reports a 1-based inclusive range for the current page', async () => {
    const h = harness({ rows: rowsOf(450), fetchedCount: 450, isEnd: true });
    expect(h.stream().firstRowNumber).toBe(1);
    expect(h.stream().lastRowNumber).toBe(200);
    await h.stream().goNext();
    expect(h.stream().firstRowNumber).toBe(201);
    expect(h.stream().lastRowNumber).toBe(400);
    h.dispose();
  });

  it('clamps the last row number to fetchedCount on a partial page', async () => {
    const h = harness({ rows: rowsOf(450), fetchedCount: 450, isEnd: true });
    await h.stream().goNext();
    await h.stream().goNext();
    expect(h.stream().lastRowNumber).toBe(450);
    h.dispose();
  });
});
