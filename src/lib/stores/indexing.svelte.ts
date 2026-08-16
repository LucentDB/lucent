import { listen } from '@tauri-apps/api/event';

export interface IndexingProgress {
  connectionId: string;
  stage: string;
  processedTables: number;
  totalTables: number;
  cacheHits: number;
  embeddingsComputed: number;
  isComplete: boolean;
  elapsedMs: number;
  detail?: string | null;
}

const SHOW_DEBOUNCE_MS = 400;
const HIDE_AFTER_COMPLETE_MS = 1500;

export const indexing = $state({
  visible: false,
  text: '',
  percent: 0,
  connections: 0,
  byConnection: new Map<string, IndexingProgress>(),
});

let showTimer: ReturnType<typeof setTimeout> | undefined;
let hideTimer: ReturnType<typeof setTimeout> | undefined;

function refresh() {
  const active = [...indexing.byConnection.values()].filter(
    (p) => !p.isComplete,
  );
  indexing.connections = active.length;
  if (active.length === 0) {
    // All done: cancel any pending show (a fast complete can race the
    // debounce window) and, if anything was shown, let it linger briefly,
    // then hide.
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
  const total = Math.max(1, first.totalTables);
  indexing.percent = Math.min(
    100,
    Math.round((first.processedTables / total) * 100),
  );
  indexing.text = first.detail
    ? first.detail
    : `Indexing schema: ${first.processedTables}/${first.totalTables} tables (${stageLabel(first.stage)})`;
  clearTimeout(hideTimer);
  if (!indexing.visible) {
    clearTimeout(showTimer);
    showTimer = setTimeout(() => (indexing.visible = true), SHOW_DEBOUNCE_MS);
  }
}

function stageLabel(stage: string): string {
  switch (stage) {
    case 'sampling':
      return 'Sampling values…';
    case 'embedding':
      return 'Embedding columns…';
    case 'model':
      return 'Loading model…';
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
  indexing.connections = 0;
  indexing.byConnection.clear();
  clearTimeout(showTimer);
  clearTimeout(hideTimer);
}
