import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';

export interface IndexingProgress {
  connectionId: string;
  stage: string;
  processedTables: number;
  totalTables: number;
  addedTablesCount?: number;
  modifiedTablesCount?: number;
  deletedTablesCount?: number;
  unchangedTablesCount?: number;
  cacheHits: number;
  embeddingsComputed: number;
  isComplete: boolean;
  elapsedMs: number;
  detail?: string | null;
}

export interface SchemaIndexingStatus {
  hasGraph: boolean;
  tier: string | null;
  tableCount: number;
  columnCount: number;
  viewCount: number;
  builtAtUnix: number;
}

const SHOW_DEBOUNCE_MS = 400;
const HIDE_AFTER_COMPLETE_MS = 1500;

export const indexing = $state({
  visible: false,
  text: '',
  percent: 0,
  stage: '',
  connections: 0,
  isComplete: true,
  detailsOpen: false,
  lastCompletedPayload: null as IndexingProgress | null,
  delta: {
    added: 0,
    modified: 0,
    deleted: 0,
    unchanged: 0,
  },
  stats: {
    cacheHits: 0,
    embeddingsComputed: 0,
    elapsedMs: 0,
  },
  byConnection: new Map<string, IndexingProgress>(),

  toggleDetails(open?: boolean) {
    if (open !== undefined) {
      indexing.detailsOpen = open;
    } else {
      indexing.detailsOpen = !indexing.detailsOpen;
    }
  },

  async syncSchemaIndexing(forceRebuild = false): Promise<void> {
    try {
      await invoke('sync_schema_indexing', { forceRebuild });
    } catch (e) {
      console.error('[indexing] failed to trigger sync_schema_indexing:', e);
    }
  },

  async fetchStatus(): Promise<SchemaIndexingStatus | null> {
    try {
      return await invoke<SchemaIndexingStatus>('get_schema_indexing_status');
    } catch (e) {
      console.error('[indexing] failed to get schema indexing status:', e);
      return null;
    }
  },
});

let showTimer: ReturnType<typeof setTimeout> | undefined;
let hideTimer: ReturnType<typeof setTimeout> | undefined;

function refresh() {
  const active = [...indexing.byConnection.values()].filter(
    (p) => !p.isComplete,
  );
  indexing.connections = active.length;

  if (active.length === 0) {
    // All done: find the latest completed event
    const completed = [...indexing.byConnection.values()].filter(
      (p) => p.isComplete,
    );
    if (completed.length > 0) {
      const latest = completed[completed.length - 1];
      indexing.lastCompletedPayload = latest;
      indexing.isComplete = true;
      indexing.stage = 'complete';
      indexing.delta = {
        added: latest.addedTablesCount ?? 0,
        modified: latest.modifiedTablesCount ?? 0,
        deleted: latest.deletedTablesCount ?? 0,
        unchanged: latest.unchangedTablesCount ?? 0,
      };
      indexing.stats = {
        cacheHits: latest.cacheHits ?? 0,
        embeddingsComputed: latest.embeddingsComputed ?? 0,
        elapsedMs: latest.elapsedMs ?? 0,
      };
    }

    clearTimeout(showTimer);
    if (indexing.visible) {
      clearTimeout(hideTimer);
      hideTimer = setTimeout(
        () => (indexing.visible = false),
        HIDE_AFTER_COMPLETE_MS,
      );
    }
    return;
  }

  const first = active[0];
  indexing.isComplete = false;
  indexing.stage = first.stage;
  const total = Math.max(1, first.totalTables);
  indexing.percent = Math.min(
    100,
    Math.round((first.processedTables / total) * 100),
  );
  indexing.delta = {
    added: first.addedTablesCount ?? 0,
    modified: first.modifiedTablesCount ?? 0,
    deleted: first.deletedTablesCount ?? 0,
    unchanged: first.unchangedTablesCount ?? 0,
  };
  indexing.stats = {
    cacheHits: first.cacheHits ?? 0,
    embeddingsComputed: first.embeddingsComputed ?? 0,
    elapsedMs: first.elapsedMs ?? 0,
  };
  indexing.text = first.detail
    ? first.detail
    : `Indexing schema: ${first.processedTables}/${first.totalTables} tables (${stageLabel(first.stage)})`;

  clearTimeout(hideTimer);
  if (!indexing.visible) {
    clearTimeout(showTimer);
    showTimer = setTimeout(() => (indexing.visible = true), SHOW_DEBOUNCE_MS);
  }
}

export function stageLabel(stage: string): string {
  switch (stage) {
    case 'discovering':
      return 'Discovering schema…';
    case 'delta':
      return 'Analyzing delta…';
    case 'sampling':
      return 'Sampling changed values…';
    case 'embedding':
      return 'Embedding columns…';
    case 'model':
      return 'Loading model…';
    case 'complete':
      return 'Schema indexed';
    default:
      return 'Indexing…';
  }
}

export async function initIndexingListeners(): Promise<void> {
  await listen<IndexingProgress>('indexing:progress', (event) => {
    indexing.byConnection.set(event.payload.connectionId, event.payload);
    refresh();
  });
  await listen<{ connectionId: string; message: string }>(
    'indexing:error',
    (event) => {
      const { connectionId, message } = event.payload;
      indexing.byConnection.delete(connectionId);
      console.warn(`[indexing] ${connectionId}: ${message}`);
      refresh();
    },
  );
}

export function __resetForTests(): void {
  indexing.visible = false;
  indexing.text = '';
  indexing.percent = 0;
  indexing.stage = '';
  indexing.connections = 0;
  indexing.isComplete = true;
  indexing.detailsOpen = false;
  indexing.lastCompletedPayload = null;
  indexing.delta = { added: 0, modified: 0, deleted: 0, unchanged: 0 };
  indexing.stats = { cacheHits: 0, embeddingsComputed: 0, elapsedMs: 0 };
  indexing.byConnection.clear();
  clearTimeout(showTimer);
  clearTimeout(hideTimer);
}
