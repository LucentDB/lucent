import { describe, it, expect } from 'vitest';
import {
  sortSpecFor,
  filterSpecFor,
  fetchMoreOptions,
  refetchOptions,
} from './tabQuery.js';

describe('sortSpecFor', () => {
  it('returns null when the tab has no sort column set', () => {
    expect(sortSpecFor({ sortCol: null, sortDir: 'asc' })).toBeNull();
  });

  it('returns a column/direction pair when a sort is set', () => {
    expect(sortSpecFor({ sortCol: 'created_at', sortDir: 'desc' })).toEqual({
      column: 'created_at',
      direction: 'desc',
    });
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
      sortCol: null,
      sortDir: 'asc',
      filters: [],
    };
    expect(fetchMoreOptions(tab, 200)).toEqual({
      limit: 200,
      offset: 400,
      sort: null,
      filters: [],
    });
  });

  it("carries the tab's current sort and filters forward unchanged", () => {
    const tab = {
      fetchedCount: 200,
      sortCol: 'id',
      sortDir: 'desc',
      filters: [{ column: 'active', operator: 'eq', value: 'true' }],
    };
    expect(fetchMoreOptions(tab, 200)).toEqual({
      limit: 200,
      offset: 200,
      sort: { column: 'id', direction: 'desc' },
      filters: [{ column: 'active', operator: 'eq', value: 'true' }],
    });
  });
});

describe('refetchOptions', () => {
  it('always resets offset to 0, regardless of how much was already fetched', () => {
    const tab = {
      fetchedCount: 800,
      sortCol: 'name',
      sortDir: 'asc',
      filters: [],
    };
    expect(refetchOptions(tab, 200)).toEqual({
      limit: 200,
      offset: 0,
      sort: { column: 'name', direction: 'asc' },
      filters: [],
    });
  });

  it('reflects a just-changed sort/filter that has not been applied to fetchedCount yet', () => {
    const tab = {
      fetchedCount: 600,
      sortCol: 'new_column',
      sortDir: 'asc',
      filters: [],
    };
    expect(refetchOptions(tab, 200).offset).toBe(0);
  });
});
