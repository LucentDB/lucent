// Pure filter logic for the results grid.
//
// Deliberately dependency-free: no Svelte, no DOM, no IPC. Every export is a
// pure function returning new values, so the interesting rules are testable
// without mounting anything. Same pattern as virtualRange.js in this folder.
//
// A filter is { id, column, operator, value }. `id` is frontend-only and is
// stripped before the filter reaches the backend — see filterSpecFor in
// src/lib/stores/tabQuery.js. Two filters may share a column; that is how a
// range (created_at >= X AND created_at <= Y) is expressed.

const NUMBER_TYPES = new Set([
  'int2',
  'int4',
  'int8',
  'float4',
  'float8',
  'numeric',
  'money',
]);

const TEMPORAL_TYPES = new Set([
  'date',
  'timestamp',
  'timestamptz',
  'time',
  'timetz',
]);

/**
 * Maps a Postgres type name (as produced by tokio_postgres Type::name(), e.g.
 * "int4", "timestamptz") onto the operator set that makes sense for it.
 * Unknown types fall back to 'text', which is safe because the text operators
 * cast with ::text.
 */
export function bucketFor(typeName) {
  const t = String(typeName || '').toLowerCase();
  if (t === 'bool') return 'bool';
  if (NUMBER_TYPES.has(t)) return 'number';
  if (TEMPORAL_TYPES.has(t)) return 'temporal';
  return 'text';
}

// The first entry in each list is that bucket's default operator.
const OPERATORS = {
  text: [
    { value: 'contains', label: 'contains', needsValue: true },
    { value: 'ncontains', label: 'does not contain', needsValue: true },
    { value: 'eq', label: 'is', needsValue: true },
    { value: 'neq', label: 'is not', needsValue: true },
    { value: 'starts', label: 'starts with', needsValue: true },
    { value: 'ends', label: 'ends with', needsValue: true },
    { value: 'null', label: 'is null', needsValue: false },
    { value: 'notnull', label: 'is not null', needsValue: false },
  ],
  number: [
    { value: 'eq', label: '=', needsValue: true },
    { value: 'neq', label: '≠', needsValue: true },
    { value: 'gt', label: '>', needsValue: true },
    { value: 'gte', label: '≥', needsValue: true },
    { value: 'lt', label: '<', needsValue: true },
    { value: 'lte', label: '≤', needsValue: true },
    { value: 'null', label: 'is null', needsValue: false },
    { value: 'notnull', label: 'is not null', needsValue: false },
  ],
  temporal: [
    { value: 'eq', label: 'is', needsValue: true },
    { value: 'neq', label: 'is not', needsValue: true },
    { value: 'gt', label: 'after', needsValue: true },
    { value: 'gte', label: 'on or after', needsValue: true },
    { value: 'lt', label: 'before', needsValue: true },
    { value: 'lte', label: 'on or before', needsValue: true },
    { value: 'null', label: 'is null', needsValue: false },
    { value: 'notnull', label: 'is not null', needsValue: false },
  ],
  bool: [
    { value: 'istrue', label: 'is true', needsValue: false },
    { value: 'isfalse', label: 'is false', needsValue: false },
    { value: 'null', label: 'is null', needsValue: false },
    { value: 'notnull', label: 'is not null', needsValue: false },
  ],
};

const NO_VALUE_OPERATORS = new Set(['null', 'notnull', 'istrue', 'isfalse']);

const EQUALITY_FAMILY = new Set(['eq', 'neq']);
const NULLCHECK_FAMILY = new Set(['null', 'notnull']);
const TRUTHINESS_FAMILY = new Set(['istrue', 'isfalse']);

export function operatorsFor(typeName) {
  return OPERATORS[bucketFor(typeName)];
}

export function defaultOperatorFor(typeName) {
  return operatorsFor(typeName)[0].value;
}

export function needsValue(operator) {
  return !NO_VALUE_OPERATORS.has(operator);
}

/**
 * Human-readable label for an operator on a column of this type, e.g. `gt` is
 * ">" on a number but "after" on a timestamp. Falls back to the raw operator
 * so an unrecognized value renders as itself rather than as blank.
 */
export function operatorLabel(operator, typeName) {
  const match = operatorsFor(typeName).find((o) => o.value === operator);
  return match ? match.label : operator;
}

/** Placeholder text for a chip's value input, hinting at an accepted format. */
export function valuePlaceholderFor(typeName) {
  switch (bucketFor(typeName)) {
    case 'number':
      return '0';
    case 'temporal':
      return '2026-01-31';
    default:
      return 'value…';
  }
}

/**
 * A filter is complete when it can be turned into a meaningful predicate.
 * Incomplete ("pending") filters stay in the UI but never reach the backend,
 * so adding a filter costs no query.
 */
export function isComplete(filter) {
  if (!filter || !filter.column) return false;
  if (!needsValue(filter.operator)) return true;
  return typeof filter.value === 'string' && filter.value.length > 0;
}

export function applyable(filters) {
  return (filters || []).filter(isComplete);
}

function makeId() {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  return `f-${Math.random().toString(36).slice(2)}`;
}

/** Assigns ids to filters that arrived without one (e.g. restored tab state). */
export function normalize(filters) {
  return (filters || []).map((f) => (f.id ? f : { ...f, id: makeId() }));
}

export function addFilter(filters, column, typeName) {
  const operator = defaultOperatorFor(typeName);
  return [
    ...(filters || []),
    {
      id: makeId(),
      column,
      operator,
      value: needsValue(operator) ? '' : null,
    },
  ];
}

export function updateFilter(filters, id, patch) {
  return (filters || []).map((f) => {
    if (f.id !== id) return f;
    const next = { ...f, ...patch };
    // Keep value and operator coherent: a valueless operator carries no value,
    // and switching back to a value-taking one re-pends the chip.
    if (!needsValue(next.operator)) return { ...next, value: null };
    if (next.value === null || next.value === undefined) {
      return { ...next, value: '' };
    }
    return next;
  });
}

export function removeFilter(filters, id) {
  return (filters || []).filter((f) => f.id !== id);
}

/**
 * Replaces an existing filter of the same family on this column, or appends.
 * Without the replacement, right-clicking two different cells in one column
 * would build `col = 'a' AND col = 'b'` and return nothing, reading as a bug.
 */
function upsertOnColumn(filters, column, operator, value, family) {
  const list = filters || [];
  const existing = list.find(
    (f) => f.column === column && family.has(f.operator),
  );
  if (existing) {
    return list.map((f) =>
      f.id === existing.id ? { ...f, operator, value } : f,
    );
  }
  return [...list, { id: makeId(), column, operator, value }];
}

/** Builds the filter for "Filter by this value" on a grid cell. */
export function filterByCellValue(
  filters,
  column,
  typeName,
  value,
  { negate = false } = {},
) {
  if (value === null || value === undefined) {
    return upsertOnColumn(
      filters,
      column,
      negate ? 'notnull' : 'null',
      null,
      NULLCHECK_FAMILY,
    );
  }

  if (bucketFor(typeName) === 'bool') {
    const wantTrue = (value === true || value === 'true') !== negate;
    return upsertOnColumn(
      filters,
      column,
      wantTrue ? 'istrue' : 'isfalse',
      null,
      TRUTHINESS_FAMILY,
    );
  }

  return upsertOnColumn(
    filters,
    column,
    negate ? 'neq' : 'eq',
    String(value),
    EQUALITY_FAMILY,
  );
}
