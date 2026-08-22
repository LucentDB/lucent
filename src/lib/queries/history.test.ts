import { describe, it, expect, vi, beforeEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  Channel: class {},
}));

import { QueryClient } from '@tanstack/svelte-query';
import { historyOptions, toggleFavorite, groupByDate } from './history.ts';

const ENTRY = {
  id: 'h1',
  connectionId: 'p1',
  connectionName: 'local',
  database: 'app',
  sql: 'select 1',
  durationMs: 3,
  rowCount: 1,
  status: 'success' as const,
  error: null,
  executedAt: '2026-08-22T10:00:00Z',
  favorite: false,
  dateGroup: 'Today',
};

const NO_FILTER = { connectionId: null, search: null, favoriteOnly: false };

function client() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
}

function callsTo(cmd: string) {
  return invoke.mock.calls.filter(([c]) => c === cmd).length;
}

beforeEach(() => {
  invoke.mockReset();
});

describe('historyOptions', () => {
  it('sends null for an empty search and a false favouritesOnly', async () => {
    invoke.mockResolvedValue([ENTRY]);
    await client().fetchQuery(historyOptions({ ...NO_FILTER, search: '' }));
    expect(invoke).toHaveBeenCalledWith('list_history', {
      connectionId: null,
      search: null,
      favoriteOnly: null,
    });
  });

  it('passes a non-empty search through', async () => {
    invoke.mockResolvedValue([]);
    await client().fetchQuery(historyOptions({ ...NO_FILTER, search: 'select' }));
    expect(invoke).toHaveBeenCalledWith('list_history', {
      connectionId: null,
      search: 'select',
      favoriteOnly: null,
    });
  });

  it('caches per filter — two different filters are two fetches', async () => {
    invoke.mockResolvedValue([]);
    const c = client();
    await c.fetchQuery(historyOptions(NO_FILTER));
    await c.fetchQuery(historyOptions({ ...NO_FILTER, favoriteOnly: true }));
    expect(callsTo('list_history')).toBe(2);
  });

  it('serves a second read of the same filter from cache', async () => {
    invoke.mockResolvedValue([]);
    const c = client();
    await c.fetchQuery(historyOptions(NO_FILTER));
    await c.fetchQuery(historyOptions(NO_FILTER));
    expect(callsTo('list_history')).toBe(1);
  });
});

describe('toggleFavorite', () => {
  it('invalidates every history filter, not just the active one', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_history') return [ENTRY];
      if (cmd === 'toggle_history_favorite') return null;
      throw new Error(`unexpected ${cmd}`);
    });
    const c = client();
    // Two distinct filters cached: favouriting must restale both, since the
    // entry moves in and out of the favourites-only list.
    await c.fetchQuery(historyOptions(NO_FILTER));
    await c.fetchQuery(historyOptions({ ...NO_FILTER, favoriteOnly: true }));
    expect(callsTo('list_history')).toBe(2);

    await toggleFavorite(c, { id: 'h1' });

    await c.fetchQuery(historyOptions(NO_FILTER));
    await c.fetchQuery(historyOptions({ ...NO_FILTER, favoriteOnly: true }));
    expect(callsTo('list_history')).toBe(4);
  });

  it('a failed toggle does not invalidate the cached lists', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_history') return [ENTRY];
      if (cmd === 'toggle_history_favorite') throw 'ipc down';
      throw new Error(`unexpected ${cmd}`);
    });
    const c = client();
    await c.fetchQuery(historyOptions(NO_FILTER));
    await expect(toggleFavorite(c, { id: 'h1' })).rejects.toBeTruthy();
    expect(c.getQueryState(['history', NO_FILTER])?.isInvalidated).toBe(false);
  });
});

describe('deleteHistoryEntry', () => {
  it('calls delete_history_entry and invalidates the prefix', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_history') return [ENTRY];
      if (cmd === 'delete_history_entry') return null;
      throw new Error(`unexpected ${cmd}`);
    });
    const c = client();
    await c.fetchQuery(historyOptions(NO_FILTER));

    const { deleteHistoryEntry } = await import('./history.ts');
    await deleteHistoryEntry(c, { id: 'h1' });

    await c.fetchQuery(historyOptions(NO_FILTER));
    expect(callsTo('delete_history_entry')).toBe(1);
    expect(callsTo('list_history')).toBe(2);
  });
});

describe('clearHistory', () => {
  it('calls clear_history and invalidates the prefix', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_history') return [];
      if (cmd === 'clear_history') return null;
      throw new Error(`unexpected ${cmd}`);
    });
    const c = client();
    await c.fetchQuery(historyOptions(NO_FILTER));

    const { clearHistory } = await import('./history.ts');
    await clearHistory(c);

    await c.fetchQuery(historyOptions(NO_FILTER));
    expect(callsTo('clear_history')).toBe(1);
    expect(callsTo('list_history')).toBe(2);
  });
});

describe('groupByDate', () => {
  it('orders known buckets before older ones', () => {
    const groups = groupByDate([
      { ...ENTRY, id: 'a', dateGroup: 'August 2026' },
      { ...ENTRY, id: 'b', dateGroup: 'Today' },
      { ...ENTRY, id: 'c', dateGroup: 'Yesterday' },
    ]);
    expect(groups.map((g) => g.label)).toEqual(['Today', 'Yesterday', 'August 2026']);
  });

  it('keeps insertion order inside a bucket', () => {
    const groups = groupByDate([
      { ...ENTRY, id: 'first', dateGroup: 'Today' },
      { ...ENTRY, id: 'second', dateGroup: 'Today' },
    ]);
    expect(groups[0].entries.map((e) => e.id)).toEqual(['first', 'second']);
  });

  it('returns an empty array for no entries', () => {
    expect(groupByDate([])).toEqual([]);
  });
});
