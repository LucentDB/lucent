import { invoke, Channel } from '@tauri-apps/api/core';
import type {
  NotebookFileV2,
  CellOutput,
  CellModel,
  NotebookEvent,
  NotebookMetadata,
  ResolvedQuery,
  TableOutput,
} from '../stores/notebook.svelte.ts';

export async function notebookOpen(path: string): Promise<NotebookFileV2> {
  return invoke('notebook_open', { path });
}

export async function notebookSave(
  sessionKey: string,
  path: string,
  metadata: NotebookMetadata,
  cells: CellModel[],
): Promise<string> {
  return invoke('notebook_save', { sessionKey, path, metadata, cells });
}

export async function notebookAttach(
  filePath: string | null,
  profileId: string,
  database: string,
): Promise<string> {
  return invoke('notebook_attach', { filePath, profileId, database });
}

export async function notebookDetach(sessionKey: string): Promise<void> {
  return invoke('notebook_detach', { sessionKey });
}

export async function notebookRestartSession(
  sessionKey: string,
): Promise<void> {
  return invoke('notebook_restart_session', { sessionKey });
}

export async function notebookRunCell(
  sessionKey: string,
  cellId: string,
  cells: CellModel[],
  channel: Channel<NotebookEvent>,
): Promise<CellOutput> {
  return invoke('notebook_run_cell', { sessionKey, cellId, cells, channel });
}

export async function notebookClearOutputs(sessionKey: string): Promise<void> {
  return invoke('notebook_clear_outputs', { sessionKey });
}

export async function notebookCancelCell(
  sessionKey: string,
  cellId: string,
): Promise<void> {
  return invoke('notebook_cancel_cell', { sessionKey, cellId });
}

export async function notebookResolveRefs(
  sessionKey: string,
  cellId: string,
  cells: CellModel[],
): Promise<ResolvedQuery> {
  return invoke('notebook_resolve_refs', { sessionKey, cellId, cells });
}

export interface SortSpec {
  column: string;
  direction: 'asc' | 'desc';
}

export interface FilterSpec {
  column: string;
  operator: string;
  value: string | null;
}

export async function notebookFetchPage(
  sessionKey: string,
  cellId: string,
  cells: CellModel[],
  limit: number,
  offset: number,
  sort: SortSpec | null,
  filters: FilterSpec[],
): Promise<TableOutput> {
  return invoke('notebook_fetch_page', {
    sessionKey,
    cellId,
    cells,
    limit,
    offset,
    sort,
    filters,
  });
}

export async function notebookCountRows(
  sessionKey: string,
  cellId: string,
  cells: CellModel[],
  filters: FilterSpec[],
): Promise<number> {
  return invoke('notebook_count_rows', { sessionKey, cellId, cells, filters });
}

export async function checkConnectionMatch(
  file: NotebookFileV2,
): Promise<boolean> {
  if (!file.metadata.connectionId) return false;
  try {
    const connections = (await invoke('list_connections')) as any[];
    return connections.some((c: any) => c.id === file.metadata.connectionId);
  } catch {
    return false;
  }
}
