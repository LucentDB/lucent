import { describe, it, expect, vi, beforeEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  Channel: class {},
}));

import { QueryClient } from '@tanstack/svelte-query';
import {
  databasesOptions,
  schemasOptions,
  objectsOptions,
  refreshExplorer,
} from './explorer.ts';

const CONN = 'p1';

/**
 * Fresh client per test so one test's cache cannot satisfy another's fetch.
 * `staleTime: Infinity` makes the refetch-count assertions causal: cached data
 * never goes stale on its own, so a second fetch that hits the wire proves
 * refreshExplorer's prefix invalidation caused it — and an untouched branch's
 * single call proves a foreign connection's invalidation did NOT reach it.
 */
function client() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
}

function callsTo(cmd: string) {
  return invoke.mock.calls.filter(([c]) => c === cmd).length;
}

/** Routes the three catalog commands the explorer issues. */
function mockCatalog() {
  invoke.mockImplementation(async (cmd: string, args: any) => {
    if (cmd === 'get_databases') return [{ name: 'app', is_current: true }];
    if (cmd === 'get_schemas') return [{ name: 'public', path: ['public'] }];
    if (cmd === 'get_schema_objects') {
      return {
        objects: [{ name: `t_${args.namespace?.join('.')}`, kind: 'table' }],
      };
    }
    throw new Error(`unexpected ${cmd}`);
  });
}

beforeEach(() => {
  invoke.mockReset();
});

describe('explorer branch queries', () => {
  it('fetches databases for a connection', async () => {
    mockCatalog();
    const dbs = await client().fetchQuery(databasesOptions(CONN));
    expect(dbs).toEqual([{ name: 'app', is_current: true }]);
  });

  it('unwraps the objects array from the get_schema_objects envelope', async () => {
    mockCatalog();
    const objects = await client().fetchQuery(
      objectsOptions(CONN, ['public'], true),
    );
    expect(objects).toEqual([{ name: 't_public', kind: 'table' }]);
  });

  it('keys objects by namespace path, so two schemas do not share a cache entry', async () => {
    mockCatalog();
    const c = client();
    await c.fetchQuery(objectsOptions(CONN, ['public'], true));
    await c.fetchQuery(objectsOptions(CONN, ['analytics'], true));
    expect(callsTo('get_schema_objects')).toBe(2);
  });

  it('separates two connections caches', async () => {
    mockCatalog();
    const c = client();
    await c.fetchQuery(databasesOptions('a'));
    await c.fetchQuery(databasesOptions('b'));
    expect(callsTo('get_databases')).toBe(2);
  });
});

describe('refreshExplorer', () => {
  it('restales every branch of the connection with one call', async () => {
    mockCatalog();
    const c = client();
    await c.fetchQuery(databasesOptions(CONN));
    await c.fetchQuery(schemasOptions(CONN, true));
    await c.fetchQuery(objectsOptions(CONN, ['public'], true));

    await refreshExplorer(c, CONN);

    await c.fetchQuery(databasesOptions(CONN));
    await c.fetchQuery(schemasOptions(CONN, true));
    await c.fetchQuery(objectsOptions(CONN, ['public'], true));

    expect(callsTo('get_databases')).toBe(2);
    expect(callsTo('get_schemas')).toBe(2);
    expect(callsTo('get_schema_objects')).toBe(2);
  });

  it('leaves another connection untouched', async () => {
    mockCatalog();
    const c = client();
    await c.fetchQuery(databasesOptions('other'));
    await refreshExplorer(c, CONN);
    await c.fetchQuery(databasesOptions('other'));
    expect(callsTo('get_databases')).toBe(1);
  });

  it('keeps the previous tree when a refetch fails', async () => {
    mockCatalog();
    const c = client();
    const before = await c.fetchQuery(databasesOptions(CONN));

    invoke.mockRejectedValue('connection lost');
    await refreshExplorer(c, CONN);
    await c.fetchQuery(databasesOptions(CONN)).catch(() => {});

    // This is what the old snapshot path's atomic commit existed to guarantee.
    // Query gives it per branch: failed refetch, previous data retained.
    expect(c.getQueryData(['explorer', CONN, 'databases'])).toEqual(before);
  });
});
