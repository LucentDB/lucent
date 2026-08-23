import { describe, it, expect } from 'vitest';
import {
  sortSpecFor,
  wireSortFor,
  filterSpecFor,
  fetchMoreOptions,
  refetchOptions,
} from './tabQuery.js';

const WIRE_COLUMNS = [
  { name: 'id', type_name: 'int4' },
  { name: 'email', type_name: 'text' },
];

describe('sortSpecFor with a sorting array', () => {
  it('maps one sort key to a single-element wire array', () => {
    const tab = { sorting: [{ id: '1', desc: true }] };
    expect(sortSpecFor(tab)).toEqual([{ id: '1', desc: true }]);
  });

  it('maps an empty sorting array to an empty wire array', () => {
    expect(sortSpecFor({ sorting: [] })).toEqual([]);
  });

  it('treats a missing sorting field as unsorted', () => {
    expect(sortSpecFor({})).toEqual([]);
  });
});

describe('sortSpecFor with multiple keys', () => {
  it('preserves every key and its order', () => {
    const tab = {
      sorting: [
        { column: 'status', direction: 'asc' },
        { column: 'created_at', direction: 'desc' },
      ],
    };
    expect(sortSpecFor(tab)).toEqual([
      { column: 'status', direction: 'asc' },
      { column: 'created_at', direction: 'desc' },
    ]);
  });

  it('is what fetchMoreOptions forwards, untruncated', () => {
    const tab = {
      fetchedCount: 200,
      filters: [],
      sorting: [
        { column: 'a', direction: 'asc' },
        { column: 'b', direction: 'desc' },
      ],
    };
    // The regression this guards: phase ② sent sort[0] because the wire took
    // one key. If that truncation survives, the second key silently vanishes.
    expect(fetchMoreOptions(tab, 200).sort).toHaveLength(2);
  });
});

describe('wireSortFor', () => {
  it('maps positional ids back to column names, in order', () => {
    expect(
      wireSortFor(
        [{ id: '1', desc: true }, { id: '0', desc: false }],
        WIRE_COLUMNS,
      ),
    ).toEqual([
      { column: 'email', direction: 'desc' },
      { column: 'id', direction: 'asc' },
    ]);
  });

  it('maps an empty or missing sorting list to an empty wire array', () => {
    expect(wireSortFor([], WIRE_COLUMNS)).toEqual([]);
    expect(wireSortFor(undefined, WIRE_COLUMNS)).toEqual([]);
  });

  it('falls back to the raw id when no column carries it', () => {
    expect(wireSortFor([{ id: '7', desc: true }], WIRE_COLUMNS)).toEqual([
      { column: '7', direction: 'desc' },
    ]);
  });

  it('resolves a REAL tab sorting into the SortSpec ARRAY the IPC takes', () => {
    // Regression: tabs hold engine SortState ({id, desc}); forwarding entries
    // verbatim fails Rust serde, which needs {column, direction}. This is the
    // exact composition App.svelte uses before invoke() — full list, no
    // truncation (phase ③ widened SortSpec to a list).
    const tab = {
      fetchedCount: 200,
      sorting: [{ id: '1', desc: true }, { id: '0', desc: false }],
      columns: WIRE_COLUMNS,
      filters: [],
    };
    const opts = {
      ...fetchMoreOptions(tab, 200),
      sort: wireSortFor(tab.sorting, tab.columns),
    };
    expect(opts.sort).toEqual([
      { column: 'email', direction: 'desc' },
      { column: 'id', direction: 'asc' },
    ]);
  });
});

describe('filterSpecFor', () => {
  it('returns an empty array when the tab has no filters', () => {
    expect(filterSpecFor({ filters: [] })).toEqual([]);
  });

  it('returns an empty array when filters is undefined', () => {
    expect(filterSpecFor({})).toEqual([]);
  });

  it('maps each filter to column/operator/value', () => {
    const tab = {
      filters: [{ column: 'active', operator: 'eq', value: 'true' }],
    };
    expect(filterSpecFor(tab)).toEqual([
      { column: 'active', operator: 'eq', value: 'true' },
    ]);
  });

  it('omits a filter whose operator needs a value but has none', () => {
    const tab = {
      filters: [
        { id: 'a', column: 'name', operator: 'contains', value: '' },
        { id: 'b', column: 'age', operator: 'gte', value: '30' },
      ],
    };
    expect(filterSpecFor(tab)).toEqual([
      { column: 'age', operator: 'gte', value: '30' },
    ]);
  });

  it('keeps valueless operators and sends a null value', () => {
    const tab = {
      filters: [
        { id: 'a', column: 'deleted_at', operator: 'null', value: null },
      ],
    };
    expect(filterSpecFor(tab)).toEqual([
      { column: 'deleted_at', operator: 'null', value: null },
    ]);
  });

  it('strips the frontend-only id', () => {
    const tab = {
      filters: [{ id: 'a', column: 'x', operator: 'eq', value: '1' }],
    };
    expect(filterSpecFor(tab)[0]).not.toHaveProperty('id');
  });

  it('emits both filters when one column is filtered twice for a range', () => {
    const tab = {
      filters: [
        { id: 'a', column: 'created_at', operator: 'gte', value: '2026-01-01' },
        { id: 'b', column: 'created_at', operator: 'lte', value: '2026-12-31' },
      ],
    };
    expect(filterSpecFor(tab)).toHaveLength(2);
  });
});

describe('fetchMoreOptions', () => {
  it("continues from the tab's current fetchedCount as the offset", () => {
    const tab = {
      fetchedCount: 400,
      sorting: [],
      filters: [],
    };
    expect(fetchMoreOptions(tab, 200)).toEqual({
      limit: 200,
      offset: 400,
      sort: [],
      filters: [],
    });
  });

  it("carries the tab's current sort and filters forward unchanged", () => {
    const tab = {
      fetchedCount: 200,
      sorting: [{ id: '1', desc: true }],
      filters: [{ column: 'active', operator: 'eq', value: 'true' }],
    };
    expect(fetchMoreOptions(tab, 200)).toEqual({
      limit: 200,
      offset: 200,
      // Raw engine state — App.svelte resolves it through wireSortFor before invoke().
      sort: [{ id: '1', desc: true }],
      filters: [{ column: 'active', operator: 'eq', value: 'true' }],
    });
  });
});

describe('refetchOptions', () => {
  it('always resets offset to 0, regardless of how much was already fetched', () => {
    const tab = {
      fetchedCount: 800,
      sorting: [{ id: '0', desc: false }],
      filters: [],
    };
    expect(refetchOptions(tab, 200)).toEqual({
      limit: 200,
      offset: 0,
      sort: [{ id: '0', desc: false }],
      filters: [],
    });
  });

  it('reflects a just-changed sort/filter that has not been applied to fetchedCount yet', () => {
    const tab = {
      fetchedCount: 600,
      sorting: [{ id: '2', desc: false }],
      filters: [],
    };
    expect(refetchOptions(tab, 200).offset).toBe(0);
  });
});
