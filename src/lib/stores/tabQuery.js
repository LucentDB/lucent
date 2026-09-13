import { applyable, needsValue } from '../grid/filters.js';

/**
 * The wire `sort` value. An array from here on: phase ③ makes it multi-key,
 * and shipping the array shape now means the payload type changes once.
 */
export function sortSpecFor(tab) {
  return tab.sorting ?? [];
}

/**
 * The ONE id→name mapper from engine SortState to the IPC SortSpec shape.
 * Sorting entries carry positional column ids; Rust's SortSpec needs
 * {column, direction}. Every IPC boundary resolves through this — do not
 * inline a second copy.
 * @typedef {{ column: string, direction: 'asc' | 'desc' }} WireSort
 * @param {{ id: string, desc: boolean }[] | undefined} sorting
 * @param {{ name: string }[]} columns
 * @returns {WireSort[]}
 */
export function wireSortFor(sorting, columns) {
  return (sorting ?? []).map((s) => ({
    column: columns[Number(s.id)]?.name ?? s.id,
    direction: s.desc ? 'desc' : 'asc',
  }));
}

export function filterSpecFor(tab) {
  return applyable(tab.filters).map((f) => ({
    column: f.column,
    operator: f.operator,
    value: needsValue(f.operator) ? f.value : null,
  }));
}

/**
 * Pagination options for appending the next chunk.
 *
 * Deliberately NO `sort` field: a raw engine SortState here is wrong for the
 * IPC wire ({column, direction}), and embedding it made correctness depend on
 * every caller remembering to override. Callers pass `sort: wireSortFor(...)`
 * explicitly — see App.svelte's executeQuery options.
 */
export function fetchMoreOptions(tab, chunkSize) {
  return {
    limit: chunkSize,
    offset: tab.fetchedCount,
    filters: filterSpecFor(tab),
  };
}

/** Same contract as fetchMoreOptions, restarting at offset 0. */
export function refetchOptions(tab, chunkSize) {
  return {
    limit: chunkSize,
    offset: 0,
    filters: filterSpecFor(tab),
  };
}
