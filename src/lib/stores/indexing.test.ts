// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  indexing,
  initIndexingListeners,
  __resetForTests,
} from './indexing.svelte';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, handler: (e: { payload: unknown }) => void) => {
    (globalThis as any).__listeners[event] = handler;
    return Promise.resolve(() => {});
  }),
}));

beforeEach(() => {
  (globalThis as any).__listeners = {};
  __resetForTests();
  vi.useFakeTimers();
});
afterEach(() => vi.useRealTimers());

describe('indexing store', () => {
  it('debounces visibility and clears on complete', async () => {
    await initIndexingListeners();
    const handler = (globalThis as any).__listeners['indexing:progress'];
    handler({
      payload: {
        connectionId: 'c1',
        stage: 'sampling',
        processedTables: 5,
        totalTables: 200,
        isComplete: false,
        elapsedMs: 10,
      },
    });
    expect(indexing.visible).toBe(false); // debounce window
    vi.advanceTimersByTime(400);
    expect(indexing.visible).toBe(true);
    expect(indexing.text).toContain('5/200');
    handler({
      payload: {
        connectionId: 'c1',
        stage: 'complete',
        processedTables: 200,
        totalTables: 200,
        isComplete: true,
        elapsedMs: 900,
      },
    });
    // The brief's original `advanceTimersByTime(50)` cannot pass against its
    // own store code: HIDE_AFTER_COMPLETE_MS is 1500 (a brief completion fade,
    // per spec §B.6). Advance through the full hide window instead.
    vi.advanceTimersByTime(1500);
    expect(indexing.visible).toBe(false);
  });

  it('never flashes for instant cached reconnects', async () => {
    await initIndexingListeners();
    const handler = (globalThis as any).__listeners['indexing:progress'];
    handler({
      payload: {
        connectionId: 'c1',
        stage: 'complete',
        processedTables: 200,
        totalTables: 200,
        isComplete: true,
        elapsedMs: 3,
      },
    });
    vi.advanceTimersByTime(400);
    expect(indexing.visible).toBe(false);
  });

  it('never shows when a fast complete races the debounce window', async () => {
    await initIndexingListeners();
    const handler = (globalThis as any).__listeners['indexing:progress'];
    // A slow-path run that finishes within the 400ms show-debounce: the
    // non-complete event schedules a show, then complete arrives before it
    // fires. The pending show must be cancelled, not fire stale afterwards.
    handler({
      payload: {
        connectionId: 'c1',
        stage: 'sampling',
        processedTables: 2,
        totalTables: 200,
        isComplete: false,
        elapsedMs: 50,
      },
    });
    handler({
      payload: {
        connectionId: 'c1',
        stage: 'complete',
        processedTables: 200,
        totalTables: 200,
        isComplete: true,
        elapsedMs: 120,
      },
    });
    vi.advanceTimersByTime(400);
    expect(indexing.visible).toBe(false);
    // And even well past the hide window, it must never have appeared.
    vi.advanceTimersByTime(1500);
    expect(indexing.visible).toBe(false);
  });
});
