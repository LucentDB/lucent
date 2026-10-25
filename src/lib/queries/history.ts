import { invoke } from '@tauri-apps/api/core';
import {
  createMutation,
  createQuery,
  queryOptions,
  useQueryClient,
  type QueryClient,
} from '@tanstack/svelte-query';
import { qk, type HistoryFilter } from './keys.ts';
import type { HistoryEntry } from '../stores/history.svelte.ts';

export type { HistoryEntry, HistoryFilter };

export function historyOptions(filter: HistoryFilter) {
  return queryOptions({
    queryKey: qk.history(filter),
    queryFn: () =>
      invoke<HistoryEntry[]>('list_history', {
        connectionId: filter.connectionId,
        // The backend treats null as "no constraint". Preserve today's wire
        // shape exactly: empty search and false favouritesOnly both send null.
        search: filter.search || null,
        favoriteOnly: filter.favoriteOnly || null,
      }),
  });
}

/**
 * Takes a getter, not a value. A plain object would freeze the key at its first
 * value and typing in the search box would never refetch.
 */
export function historyQuery(getFilter: () => HistoryFilter) {
  return createQuery(() => historyOptions(getFilter()));
}

// ─── Mutations ──────────────────────────────────────────────────────────────
// All three invalidate the ['history'] PREFIX, not one filter's key. An entry
// changing favourite state moves it in and out of the favourites-only list, so
// every cached filter combination is affected.

function invalidateAll(client: QueryClient) {
  return client.invalidateQueries({ queryKey: ['history'] });
}

export async function toggleFavorite(
  client: QueryClient,
  vars: { id: string },
) {
  await invoke('toggle_history_favorite', { id: vars.id });
  await invalidateAll(client);
}

export async function deleteHistoryEntry(
  client: QueryClient,
  vars: { id: string },
) {
  await invoke('delete_history_entry', { id: vars.id });
  await invalidateAll(client);
}

export async function clearHistory(client: QueryClient) {
  await invoke('clear_history');
  await invalidateAll(client);
}

export function toggleFavoriteMutation() {
  const client = useQueryClient();
  return createMutation(() => ({
    mutationFn: (vars: { id: string }) => toggleFavorite(client, vars),
  }));
}

export function deleteHistoryEntryMutation() {
  const client = useQueryClient();
  return createMutation(() => ({
    mutationFn: (vars: { id: string }) => deleteHistoryEntry(client, vars),
  }));
}

export function clearHistoryMutation() {
  const client = useQueryClient();
  return createMutation(() => ({
    mutationFn: () => clearHistory(client),
  }));
}

// ─── Grouping ───────────────────────────────────────────────────────────────

export interface HistoryGroup {
  label: string;
  entries: HistoryEntry[];
}

const KNOWN_ORDER = ['Today', 'Yesterday', 'This Week', 'Last Week'];

/**
 * Buckets entries by their backend-assigned `dateGroup`. Known buckets come
 * first in calendar order; anything else follows in insertion order.
 */
export function groupByDate(entries: HistoryEntry[]): HistoryGroup[] {
  const grouped = new Map<string, HistoryEntry[]>();
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i];
    let list = grouped.get(e.dateGroup);
    if (!list) {
      list = [];
      grouped.set(e.dateGroup, list);
    }
    list.push(e);
  }

  const groups: HistoryGroup[] = [];
  for (let i = 0; i < KNOWN_ORDER.length; i++) {
    const label = KNOWN_ORDER[i];
    const list = grouped.get(label);
    if (list) {
      groups.push({ label, entries: list });
      grouped.delete(label);
    }
  }
  for (const [label, list] of grouped) {
    groups.push({ label, entries: list });
  }
  return groups;
}
