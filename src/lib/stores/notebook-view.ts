import * as nb from '../ipc/notebook';
import type { FilterSpec, SortSpec } from '../ipc/notebook';
import type {
  ColumnMeta,
  NotebookModel,
  TableOutput,
} from './notebook.svelte.ts';

export const CELL_PAGE_SIZES = [5, 10, 25] as const;
export const DEFAULT_CELL_PAGE_SIZE = 10;

export interface CellViewState {
  filters: FilterSpec[];
  sortCol: string | null;
  sortDir: 'asc' | 'desc';
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
    sortCol: null,
    sortDir: 'asc',
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

function sortSpec(state: CellViewState): SortSpec | null {
  return state.sortCol
    ? { column: state.sortCol, direction: state.sortDir }
    : null;
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

  async function refetch(cellId: string, state: CellViewState, offset: number) {
    if (!model.sessionKey) return;
    put(cellId, { ...state, loading: true });
    try {
      const out = await nb.notebookFetchPage(
        model.sessionKey,
        cellId,
        model.cells,
        state.pageSize,
        offset,
        sortSpec(state),
        state.filters,
      );
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
        sortCol: string | null;
        sortDir: 'asc' | 'desc';
      },
    ) {
      const next: CellViewState = {
        ...stateFor(cellId),
        filters: s.filters,
        sortCol: s.sortCol,
        sortDir: s.sortDir,
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
      states.delete(cellId);
      const cell = model.cells.find((c) => c.id === cellId);
      if (cell) cell.view = stateFor(cellId);
    },
  };
}
