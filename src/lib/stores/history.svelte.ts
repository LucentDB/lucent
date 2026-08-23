// No `invoke` import: the reduced store issues no IPC. The entries themselves
// are fetched by src/lib/queries/history.ts.

export interface HistoryEntry {
  id: string;
  connectionId: string;
  connectionName: string;
  database: string;
  sql: string;
  durationMs: number;
  rowCount: number | null;
  status: 'success' | 'error';
  error: string | null;
  executedAt: string;
  favorite: boolean;
  dateGroup: string;
}

/**
 * Filter selection only. The entries themselves live in the history query,
 * keyed by this filter — see src/lib/queries/history.ts.
 */
class HistoryFilterStore {
  searchQuery = $state('');
  filterConnectionId = $state<string | null>(null);
  showFavoritesOnly = $state(false);

  /** The shape the query key is built from. */
  get filter() {
    return {
      connectionId: this.filterConnectionId,
      search: this.searchQuery || null,
      favoriteOnly: this.showFavoritesOnly,
    };
  }

  setSearch(query: string) {
    this.searchQuery = query;
  }

  setFilterConnection(id: string | null) {
    this.filterConnectionId = id;
  }

  setFavoritesOnly(v: boolean) {
    this.showFavoritesOnly = v;
  }
}

export const history = new HistoryFilterStore();
