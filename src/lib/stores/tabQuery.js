import { applyable, needsValue } from '../components/grid/filters.js';

export function sortSpecFor(tab) {
  return tab.sortCol ? { column: tab.sortCol, direction: tab.sortDir } : null;
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
