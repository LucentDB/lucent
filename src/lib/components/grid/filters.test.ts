import { describe, it, expect } from 'vitest';
import {
  bucketFor,
  operatorsFor,
  defaultOperatorFor,
  needsValue,
  valuePlaceholderFor,
  isComplete,
  applyable,
  normalize,
  addFilter,
  updateFilter,
  removeFilter,
  filterByCellValue,
} from './filters.js';

describe('bucketFor', () => {
  it('buckets postgres integer and float types as numbers', () => {
    for (const t of [
      'int2',
      'int4',
      'int8',
      'float4',
      'float8',
      'numeric',
      'money',
    ]) {
      expect(bucketFor(t)).toBe('number');
    }
  });

  it('buckets postgres date and time types as temporal', () => {
    for (const t of ['date', 'timestamp', 'timestamptz', 'time', 'timetz']) {
      expect(bucketFor(t)).toBe('temporal');
    }
  });

  it('buckets bool as bool', () => {
    expect(bucketFor('bool')).toBe('bool');
  });

  it('buckets text-like types as text', () => {
    for (const t of ['text', 'varchar', 'bpchar', 'uuid', 'jsonb']) {
      expect(bucketFor(t)).toBe('text');
    }
  });

  it('falls back to text for unknown types, since contains casts with ::text', () => {
    expect(bucketFor('some_custom_enum')).toBe('text');
  });

  it('tolerates a missing type name', () => {
    expect(bucketFor(undefined)).toBe('text');
  });
});

describe('operatorsFor', () => {
  it('offers comparison operators to numeric columns', () => {
    const ops = operatorsFor('int4').map((o) => o.value);
    expect(ops).toEqual([
      'eq',
      'neq',
      'gt',
      'gte',
      'lt',
      'lte',
      'null',
      'notnull',
    ]);
  });

  it('labels comparisons in date language for temporal columns', () => {
    const labels = Object.fromEntries(
      operatorsFor('timestamptz').map((o) => [o.value, o.label]),
    );
    expect(labels.gt).toBe('after');
    expect(labels.lte).toBe('on or before');
  });

  it('offers substring operators to text columns', () => {
    const ops = operatorsFor('text').map((o) => o.value);
    expect(ops).toContain('contains');
    expect(ops).toContain('ncontains');
    expect(ops).toContain('ends');
    expect(ops).not.toContain('gt');
  });

  it('offers only truthiness and null checks to bool columns', () => {
    const ops = operatorsFor('bool').map((o) => o.value);
    expect(ops).toEqual(['istrue', 'isfalse', 'null', 'notnull']);
  });
});

describe('defaultOperatorFor', () => {
  it('defaults text columns to contains', () => {
    expect(defaultOperatorFor('text')).toBe('contains');
  });

  it('defaults numeric columns to equality', () => {
    expect(defaultOperatorFor('int8')).toBe('eq');
  });

  it('defaults bool columns to is true', () => {
    expect(defaultOperatorFor('bool')).toBe('istrue');
  });
});

describe('needsValue', () => {
  it('is false for the operators that are complete on their own', () => {
    for (const op of ['null', 'notnull', 'istrue', 'isfalse']) {
      expect(needsValue(op)).toBe(false);
    }
  });

  it('is true for operators that compare against something', () => {
    for (const op of [
      'eq',
      'neq',
      'contains',
      'ncontains',
      'starts',
      'ends',
      'gt',
      'lte',
    ]) {
      expect(needsValue(op)).toBe(true);
    }
  });
});

describe('valuePlaceholderFor', () => {
  it('hints at an accepted date format for temporal columns', () => {
    expect(valuePlaceholderFor('timestamptz')).toBe('2026-01-31');
  });

  it('hints at a number for numeric columns', () => {
    expect(valuePlaceholderFor('int4')).toBe('0');
  });

  it('falls back to a generic hint for text columns', () => {
    expect(valuePlaceholderFor('text')).toBe('value…');
  });
});

describe('isComplete', () => {
  it('treats a value-taking operator with no value as incomplete', () => {
    expect(
      isComplete({ column: 'name', operator: 'contains', value: '' }),
    ).toBe(false);
  });

  it('treats a value-taking operator with a value as complete', () => {
    expect(
      isComplete({ column: 'name', operator: 'contains', value: 'a' }),
    ).toBe(true);
  });

  it('treats a valueless operator as complete', () => {
    expect(isComplete({ column: 'x', operator: 'null', value: null })).toBe(
      true,
    );
  });

  it('rejects a filter with no column', () => {
    expect(isComplete({ column: '', operator: 'null', value: null })).toBe(
      false,
    );
  });

  it('treats a whitespace-only value as a real value, since SQL cares', () => {
    expect(isComplete({ column: 'name', operator: 'eq', value: ' ' })).toBe(
      true,
    );
  });
});

describe('applyable', () => {
  it('drops incomplete filters and keeps complete ones', () => {
    const filters: Array<{
      id: string;
      column: string;
      operator: string;
      value: string | null;
    }> = [
      { id: 'a', column: 'name', operator: 'contains', value: '' },
      { id: 'b', column: 'deleted_at', operator: 'null', value: null },
      { id: 'c', column: 'age', operator: 'gte', value: '30' },
    ];
    expect(applyable(filters).map((f: any) => f.id)).toEqual(['b', 'c']);
  });

  it('returns an empty array for undefined input', () => {
    expect(applyable(undefined)).toEqual([]);
  });

  it('does not mutate its input', () => {
    const filters: Array<{
      id: string;
      column: string;
      operator: string;
      value: string;
    }> = [{ id: 'a', column: 'n', operator: 'eq', value: '' }];
    const snapshot = JSON.stringify(filters);
    applyable(filters);
    expect(JSON.stringify(filters)).toBe(snapshot);
  });
});

describe('normalize', () => {
  it('assigns ids to filters restored without one', () => {
    const result = normalize([{ column: 'a', operator: 'eq', value: '1' }]);
    expect(result[0].id).toBeTruthy();
    expect(result[0].column).toBe('a');
  });

  it('preserves existing ids', () => {
    const result = normalize([
      { id: 'keep', column: 'a', operator: 'eq', value: '1' },
    ]);
    expect(result[0].id).toBe('keep');
  });

  it('gives distinct ids to distinct filters', () => {
    const result = normalize([
      { column: 'a', operator: 'eq', value: '1' },
      { column: 'a', operator: 'neq', value: '2' },
    ]);
    expect(result[0].id).not.toBe(result[1].id);
  });
});

describe('addFilter', () => {
  it('appends a pending filter with the type-appropriate default operator', () => {
    const result = addFilter([], 'name', 'text');
    expect(result).toHaveLength(1);
    expect(result[0].column).toBe('name');
    expect(result[0].operator).toBe('contains');
    expect(result[0].value).toBe('');
    expect(isComplete(result[0])).toBe(false);
  });

  it('appends an already-complete filter for a bool column', () => {
    const result = addFilter([], 'active', 'bool');
    expect(result[0].operator).toBe('istrue');
    expect(result[0].value).toBeNull();
    expect(isComplete(result[0])).toBe(true);
  });

  it('allows a second filter on a column already filtered, for ranges', () => {
    const one = addFilter([], 'created_at', 'timestamptz');
    const two = addFilter(one, 'created_at', 'timestamptz');
    expect(two).toHaveLength(2);
    expect(two[0].id).not.toBe(two[1].id);
  });

  it('does not mutate the input array', () => {
    const filters: Array<{
      id?: string;
      column?: string;
      operator?: string;
      value?: string | null;
    }> = [];
    addFilter(filters, 'name', 'text');
    expect(filters).toHaveLength(0);
  });
});

describe('updateFilter', () => {
  it('patches only the filter with the matching id', () => {
    const filters: Array<{
      id: string;
      column: string;
      operator: string;
      value: string;
    }> = [
      { id: 'a', column: 'name', operator: 'contains', value: 'x' },
      { id: 'b', column: 'name', operator: 'contains', value: 'y' },
    ];
    const result = updateFilter(filters, 'b', { value: 'z' });
    expect(result[0].value).toBe('x');
    expect(result[1].value).toBe('z');
  });

  it('clears a stale value when switching to a valueless operator', () => {
    const filters = [{ id: 'a', column: 'x', operator: 'eq', value: 'old' }];
    const result = updateFilter(filters, 'a', { operator: 'null' });
    expect(result[0].value).toBeNull();
    expect(isComplete(result[0])).toBe(true);
  });

  it('re-pends the filter when switching from a valueless to a value-taking operator', () => {
    const filters = [{ id: 'a', column: 'x', operator: 'null', value: null }];
    const result = updateFilter(filters, 'a', { operator: 'eq' });
    expect(result[0].value).toBe('');
    expect(isComplete(result[0])).toBe(false);
  });

  it('re-pends a filter whose value is deleted', () => {
    const filters = [{ id: 'a', column: 'x', operator: 'eq', value: 'v' }];
    const result = updateFilter(filters, 'a', { value: '' });
    expect(isComplete(result[0])).toBe(false);
  });

  it('returns new objects rather than mutating', () => {
    const filters = [{ id: 'a', column: 'x', operator: 'eq', value: 'v' }];
    const result = updateFilter(filters, 'a', { value: 'w' });
    expect(filters[0].value).toBe('v');
    expect(result[0]).not.toBe(filters[0]);
  });
});

describe('removeFilter', () => {
  it('removes only the filter with the matching id', () => {
    const filters = [
      { id: 'a', column: 'x', operator: 'eq', value: '1' },
      { id: 'b', column: 'x', operator: 'eq', value: '2' },
    ];
    expect(removeFilter(filters, 'a').map((f: any) => f.id)).toEqual(['b']);
  });

  it('does not mutate the input array', () => {
    const filters = [{ id: 'a', column: 'x', operator: 'eq', value: '1' }];
    removeFilter(filters, 'a');
    expect(filters).toHaveLength(1);
  });
});

describe('filterByCellValue', () => {
  it('adds an equality filter for a plain value', () => {
    const result = filterByCellValue([], 'status', 'text', 'active', {});
    expect(result[0].operator).toBe('eq');
    expect(result[0].value).toBe('active');
  });

  it('adds an inequality filter when negated', () => {
    const result = filterByCellValue([], 'status', 'text', 'active', {
      negate: true,
    });
    expect(result[0].operator).toBe('neq');
  });

  it('stringifies numeric cell values', () => {
    const result = filterByCellValue([], 'age', 'int4', 41, {});
    expect(result[0].value).toBe('41');
  });

  it('updates an existing equality filter on the column instead of appending', () => {
    const filters = [
      { id: 'a', column: 'status', operator: 'eq', value: 'old' },
    ];
    const result = filterByCellValue(filters, 'status', 'text', 'new', {});
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe('a');
    expect(result[0].value).toBe('new');
  });

  it('leaves a non-equality filter on the same column alone and appends', () => {
    const filters = [
      { id: 'a', column: 'status', operator: 'contains', value: 'act' },
    ];
    const result = filterByCellValue(filters, 'status', 'text', 'active', {});
    expect(result).toHaveLength(2);
  });

  it('uses a null check for a NULL cell, whatever the column type', () => {
    const result = filterByCellValue([], 'deleted_at', 'timestamptz', null, {});
    expect(result[0].operator).toBe('null');
    expect(result[0].value).toBeNull();
  });

  it('uses a not-null check for a negated NULL cell', () => {
    const result = filterByCellValue([], 'deleted_at', 'timestamptz', null, {
      negate: true,
    });
    expect(result[0].operator).toBe('notnull');
  });

  it('uses truthiness operators for a bool cell rather than eq on a string', () => {
    expect(filterByCellValue([], 'active', 'bool', true, {})[0].operator).toBe(
      'istrue',
    );
    expect(filterByCellValue([], 'active', 'bool', false, {})[0].operator).toBe(
      'isfalse',
    );
  });

  it('inverts the truthiness operator for a negated bool cell', () => {
    expect(
      filterByCellValue([], 'active', 'bool', true, { negate: true })[0]
        .operator,
    ).toBe('isfalse');
  });
});
