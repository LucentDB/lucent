import { describe, it, expect, vi, beforeEach } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
  Channel: class {},
}));

import { schemaSummary, pickSchemas } from './schema-summary.svelte.ts';

/** A get_schema_objects response with the given tables, plus noise. */
function objectsFor(tables: { name: string; rows?: unknown }[]) {
  return {
    name: 'x',
    objects: [
      ...tables.map((t) => ({
        name: t.name,
        kind: 'table',
        row_count: 'rows' in t ? t.rows : 0,
      })),
      { name: 'a_view', kind: 'view', row_count: null },
      { name: 'a_func', kind: 'function', row_count: null },
    ],
  };
}

/**
 * Routes the two commands the store issues. `schemas` maps a schema name to
 * the tables it contains, and drives get_schemas' counts too. The wire shape
 * carries the namespace `path` — the store passes those segments back.
 */
function mockDatabase(schemas: Record<string, { name: string }[]>) {
  invoke.mockImplementation(async (cmd: string, args: any) => {
    if (cmd === 'get_schemas') {
      return Object.entries(schemas).map(([name, tables]) => ({
        name,
        path: [name],
        object_count: tables.length,
      }));
    }
    if (cmd === 'get_schema_objects') {
      return objectsFor(schemas[args.namespace?.join('.')] ?? []);
    }
    throw new Error(`unexpected command ${cmd}`);
  });
}

function objectCalls(): string[][] {
  return invoke.mock.calls
    .filter(([cmd]) => cmd === 'get_schema_objects')
    .map(([, args]) => args.namespace);
}

beforeEach(() => {
  invoke.mockReset();
  schemaSummary.reset();
});

describe('pickSchemas', () => {
  const names = (schemas: { name: string }[]) => schemas.map((s) => s.name);

  it('prefers public when it has objects', () => {
    expect(
      names(
        pickSchemas([
          { name: 'bookings', path: ['bookings'], object_count: 9 },
          { name: 'public', path: ['public'], object_count: 2 },
        ]),
      ),
    ).toEqual(['public', 'bookings']);
  });

  it('skips empty schemas — probing them can only return nothing', () => {
    expect(
      names(
        pickSchemas([
          { name: 'public', path: ['public'], object_count: 0 },
          { name: 'bookings', path: ['bookings'], object_count: 9 },
        ]),
      ),
    ).toEqual(['bookings']);
  });

  it('orders non-public schemas by size', () => {
    expect(
      names(
        pickSchemas([
          { name: 'small', path: ['small'], object_count: 1 },
          { name: 'big', path: ['big'], object_count: 50 },
          { name: 'mid', path: ['mid'], object_count: 7 },
        ]),
      ),
    ).toEqual(['big', 'mid', 'small']);
  });

  it('probes at most three schemas', () => {
    const many = Array.from({ length: 20 }, (_, i) => ({
      name: `s${i}`,
      path: [`s${i}`],
      object_count: i + 1,
    }));
    expect(pickSchemas(many)).toHaveLength(3);
  });

  it('returns nothing when every schema is empty', () => {
    expect(
      pickSchemas([{ name: 'public', path: ['public'], object_count: 0 }]),
    ).toEqual([]);
  });
});

describe('schemaSummary', () => {
  it('keeps only base tables', async () => {
    mockDatabase({ public: [{ name: 'orders' }] });
    await schemaSummary.load('shop');

    expect(schemaSummary.tables).toEqual([{ name: 'orders', rowCount: 0 }]);
    expect(schemaSummary.schema).toBe('public');
    expect(schemaSummary.loaded).toBe(true);
  });

  it('falls through an empty public to a schema that has tables', async () => {
    // The real-world case: tables live in `bookings`, `public` is bare.
    mockDatabase({ public: [], bookings: [{ name: 'flights' }] });
    await schemaSummary.load('demo');

    expect(schemaSummary.schema).toBe('bookings');
    expect(schemaSummary.tables).toEqual([{ name: 'flights', rowCount: 0 }]);
  });

  it('stops probing as soon as it finds tables', async () => {
    mockDatabase({ public: [{ name: 'orders' }], other: [{ name: 'x' }] });
    await schemaSummary.load('shop');

    expect(objectCalls()).toEqual([['public']]);
  });

  it('coerces a non-numeric row count to null rather than NaN', async () => {
    mockDatabase({ public: [] });
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_schemas') {
        return [{ name: 'public', path: ['public'], object_count: 1 }];
      }
      return objectsFor([{ name: 'orders', rows: undefined }]);
    });
    await schemaSummary.load('shop');

    expect(schemaSummary.tables).toEqual([{ name: 'orders', rowCount: null }]);
  });

  it('caches — a repeat load for the same database does not re-query', async () => {
    mockDatabase({ public: [{ name: 'orders' }] });
    await schemaSummary.load('shop');
    await schemaSummary.load('shop');

    expect(objectCalls()).toHaveLength(1);
  });

  it('re-queries when the database changes, so suggestions never go stale', async () => {
    mockDatabase({ public: [{ name: 'orders' }] });
    await schemaSummary.load('shop');

    mockDatabase({ public: [{ name: 'ledger' }] });
    await schemaSummary.load('accounting');

    expect(schemaSummary.tables).toEqual([{ name: 'ledger', rowCount: 0 }]);
    expect(schemaSummary.database).toBe('accounting');
  });

  it('fails silently to an empty list, and still counts as loaded', async () => {
    invoke.mockRejectedValue(new Error('not connected'));
    await schemaSummary.load('shop');

    expect(schemaSummary.tables).toEqual([]);
    expect(schemaSummary.schema).toBeNull();
    expect(schemaSummary.loaded).toBe(true);
    expect(schemaSummary.loading).toBe(false);
  });

  it('records an empty result for a database with no tables anywhere', async () => {
    mockDatabase({ public: [] });
    await schemaSummary.load('bare');

    expect(schemaSummary.tables).toEqual([]);
    expect(schemaSummary.loaded).toBe(true);
  });

  it('reset() clears the cache so the next connection re-reads', async () => {
    mockDatabase({ public: [{ name: 'orders' }] });
    await schemaSummary.load('shop');
    schemaSummary.reset();

    expect(schemaSummary.tables).toEqual([]);
    expect(schemaSummary.loaded).toBe(false);
    expect(schemaSummary.database).toBeNull();

    await schemaSummary.load('shop');
    expect(objectCalls()).toHaveLength(2);
  });

  it('ignores a concurrent load rather than double-querying', async () => {
    mockDatabase({ public: [{ name: 'orders' }] });

    // load() flips `loading` synchronously before its first await, so the
    // second call must bail out immediately.
    await Promise.all([schemaSummary.load('shop'), schemaSummary.load('shop')]);

    expect(
      invoke.mock.calls.filter(([cmd]) => cmd === 'get_schemas'),
    ).toHaveLength(1);
    expect(objectCalls()).toHaveLength(1);
    expect(schemaSummary.loading).toBe(false);
  });
});
