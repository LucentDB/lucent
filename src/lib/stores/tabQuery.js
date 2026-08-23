import { applyable, needsValue } from '../grid/filters.js';

/**
 * The wire `sort` value. An array from here on: phase ③ makes it multi-key,
 * and shipping the array shape now means the payload type changes once.
 */
export function sortSpecFor(tab) {
  return tab.sorting ?? [];
}

export function filterSpecFor(tab) {
  return applyable(tab.filters).map((f) => ({
    column: f.column,
    operator: f.operator,
    value: needsValue(f.operator) ? f.value : null,
  }));
}

export function fetchMoreOptions(tab, chunkSize) {
  return {
    limit: chunkSize,
    offset: tab.fetchedCount,
    sort: sortSpecFor(tab),
    filters: filterSpecFor(tab),
  };
}

export function refetchOptions(tab, chunkSize) {
  return {
    limit: chunkSize,
    offset: 0,
    sort: sortSpecFor(tab),
    filters: filterSpecFor(tab),
  };
}
