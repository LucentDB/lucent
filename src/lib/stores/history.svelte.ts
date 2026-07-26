import { invoke } from '@tauri-apps/api/core';

// ─── Types ──────────────────────────────────────────────────────────────────

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

// ─── Store ──────────────────────────────────────────────────────────────────

class HistoryStore {
  entries = $state<HistoryEntry[]>([]);
  loading = $state(false);
  error = $state<string | null>(null);
  searchQuery = $state('');
  filterConnectionId = $state<string | null>(null);
  showFavoritesOnly = $state(false);

  /** Group entries by date group for display */
  groupedEntries = $derived.by(() => {
    const groups: { label: string; entries: HistoryEntry[] }[] = [];
    const grouped = new Map<string, HistoryEntry[]>();

    for (const e of this.entries) {
      const key = e.dateGroup;
      if (!grouped.has(key)) grouped.set(key, []);
      grouped.get(key)!.push(e);
    }

    const order = ['Today', 'Yesterday', 'This Week', 'Last Week'];
    for (const label of order) {
      if (grouped.has(label)) {
        groups.push({ label, entries: grouped.get(label)! });
        grouped.delete(label);
      }
    }
    // Remaining groups (older)
    for (const [label, entries] of grouped) {
      groups.push({ label, entries });
    }

    return groups;
  });

  async loadHistory() {
    this.loading = true;
    this.error = null;
    try {
      this.entries = await invoke<HistoryEntry[]>('list_history', {
        connectionId: this.filterConnectionId,
        search: this.searchQuery || null,
        favoriteOnly: this.showFavoritesOnly || null,
      });
    } catch (e: any) {
      this.error =
        typeof e === 'string' ? e : (e?.message ?? 'Failed to load history');
    } finally {
      this.loading = false;
    }
  }

  async toggleFavorite(id: string) {
    try {
      await invoke('toggle_history_favorite', { id });
      await this.loadHistory();
    } catch (e) {
      console.error('Failed to toggle favorite:', e);
    }
  }

  async deleteEntry(id: string) {
    try {
      await invoke('delete_history_entry', { id });
      await this.loadHistory();
    } catch (e) {
      console.error('Failed to delete history entry:', e);
    }
  }

  async clearHistory() {
    if (!confirm('Clear all query history?')) return;
    try {
      await invoke('clear_history');
      this.entries = [];
    } catch (e) {
      console.error('Failed to clear history:', e);
    }
  }

  setSearch(query: string) {
    this.searchQuery = query;
    this.loadHistory();
  }

  setFilterConnection(id: string | null) {
    this.filterConnectionId = id;
    this.loadHistory();
  }

  setFavoritesOnly(v: boolean) {
    this.showFavoritesOnly = v;
    this.loadHistory();
  }
}

export const history = new HistoryStore();
