import * as nb from '../ipc/notebook';
import type { FilterSpec } from '../ipc/notebook';
import { wireSortFor } from './tabQuery.js';
import type {
  ColumnMeta,
  NotebookModel,
  TableOutput,
} from './notebook.svelte.ts';

export const CELL_PAGE_SIZES = [5, 10, 25] as const;
export const DEFAULT_CELL_PAGE_SIZE = 10;

export interface CellViewState {
  filters: FilterSpec[];
  sorting: { id: string; desc: boolean }[];
  pageSize: number;
  columns: ColumnMeta[];
  rows: unknown[][];
  fetchedCount: number;
  totalCount: number | null;
  isEnd: boolean;
  loading: boolean;
  /** False for DML/DDL cells, which cannot be paged or filtered. */
  pageable: boolean;
}

export function defaultViewState(
  pageSize = DEFAULT_CELL_PAGE_SIZE,
): CellViewState {
  return {
    filters: [],
    sorting: [],
    pageSize,
    columns: [],
    rows: [],
    fetchedCount: 0,
    totalCount: null,
    isEnd: false,
    loading: false,
    pageable: true,
  };
}

function isTable(o: unknown): o is TableOutput {
  return !!o && typeof o === 'object' && 'columns' in o;
}

/**
 * Per-cell grid view state. Lives beside the model rather than on it because it
 * is session state: it is deliberately absent from NotebookFileCell, so it cannot
 * leak into a saved .lucent file.
 */
export function createCellView(model: NotebookModel) {
  const states = new Map<string, CellViewState>();

  function stateFor(cellId: string): CellViewState {
    let s = states.get(cellId);
    if (s) return s;

    s = defaultViewState();
    const cell = model.cells.find((c) => c.id === cellId);
    if (cell && isTable(cell.outputs)) {
      const out = cell.outputs;
      s.columns = out.columns;
      s.rows = out.rows;
      s.fetchedCount = out.rows.length;
      s.totalCount = out.total_count;
      s.pageSize = out.page_size ?? DEFAULT_CELL_PAGE_SIZE;
      s.pageable = out.is_wrappable ?? true;
      // The first run is itself a `LIMIT page_size OFFSET 0` page (see
      // run_sql_cell), so a short page proves the end — the same inference
      // refetch makes. Leaving isEnd false here made every small result claim
      // "of 1+" with live Prev/Next controls it could not honour.
      s.isEnd = !s.pageable || out.rows.length < s.pageSize;
    }
    states.set(cellId, s);
    return s;
  }

  function put(cellId: string, next: CellViewState) {
    states.set(cellId, next);
    // Mirror onto the cell so the grid re-renders through Svelte's reactivity.
    const cell = model.cells.find((c) => c.id === cellId);
    if (cell) cell.view = next;
  }

/** Temporary paging diagnostics — enable with localStorage.lucentPagingDebug. */
function debugView(event: string, detail: Record<string, unknown>) {
  if (typeof localStorage !== 'undefined' && localStorage.lucentPagingDebug) {
    console.warn(`[cellView] ${event}`, detail);
  }
}

  async function refetch(cellId: string, state: CellViewState, offset: number) {
    if (!model.sessionKey) return;
    debugView('refetch', { cellId, offset, fetchedCount: state.fetchedCount });
    put(cellId, { ...state, loading: true });
    try {
      const out = await nb.notebookFetchPage(
        model.sessionKey,
        cellId,
        model.cells,
        state.pageSize,
        offset,
        // The full sort list crosses: phase ③ widened SortSpec to a list.
        wireSortFor(state.sorting, state.columns),
        state.filters,
      );
      debugView('refetch resolved', { offset, incoming: out.rows.length, bufferAfter: offset === 0 ? out.rows.length : state.rows.length + out.rows.length });
      const rows = offset === 0 ? out.rows : [...state.rows, ...out.rows];
      put(cellId, {
        ...state,
        columns: out.columns.length ? out.columns : state.columns,
        rows,
        fetchedCount: rows.length,
        isEnd: out.rows.length < state.pageSize,
        pageable: out.is_wrappable ?? state.pageable,
        loading: false,
      });
    } catch (e) {
      console.error(`[notebook] fetch page failed for cell ${cellId}:`, e);
      put(cellId, { ...state, loading: false });
    }
  }

  return {
    stateFor,

    /** Filter or sort change: always restarts paging from offset 0. */
    async applyState(
      cellId: string,
      s: {
        filters: FilterSpec[];
        sorting: { id: string; desc: boolean }[];
      },
    ) {
      const next: CellViewState = {
        ...stateFor(cellId),
        filters: s.filters,
        sorting: s.sorting,
        totalCount: null, // a filter change invalidates any previous count
      };
      await refetch(cellId, next, 0);
    },

    async fetchMore(cellId: string) {
      const state = stateFor(cellId);
      if (state.isEnd || state.loading) return;
      await refetch(cellId, state, state.fetchedCount);
    },

    async countAll(cellId: string) {
      if (!model.sessionKey) return;
      const state = stateFor(cellId);
      try {
        const total = await nb.notebookCountRows(
          model.sessionKey,
          cellId,
          model.cells,
          state.filters,
        );
        put(cellId, { ...stateFor(cellId), totalCount: total });
      } catch (e) {
        console.error(`[notebook] count failed for cell ${cellId}:`, e);
      }
    },

    async setPageSize(cellId: string, pageSize: number) {
      const next = { ...stateFor(cellId), pageSize };
      await refetch(cellId, next, 0);
    },

    /** Called after a cell re-runs, so its window restarts from the new output. */
    resetFrom(cellId: string) {
      debugView('resetFrom', { cellId, stack: new Error().stack?.split('\n')[2]?.trim() });
      states.delete(cellId);
      const cell = model.cells.find((c) => c.id === cellId);
      if (cell) cell.view = stateFor(cellId);
    },
  };
}
