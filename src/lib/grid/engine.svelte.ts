/**
 * The ONLY module in the app that imports @tanstack/svelte-table.
 * If the adapter changes, exactly this file changes with it. See spec §3.3.
 *
 * MANUAL MODE (spec D3): the backend does the sorting, filtering and paging.
 * Table holds the state and emits changes. Registering a sorted, filtered, or
 * paginated row model here would silently operate on the fetched slice while
 * presenting it as the whole result — a correctness bug, not an optimisation.
 */
// svelte-table re-exports all of table-core (`export * from
// '@tanstack/table-core'`), so features and row models come from here too —
// which is why table-core is not a direct dependency. Spec §9.
import {
  cellSelectionFeature,
  columnFilteringFeature,
  columnPinningFeature,
  columnResizingFeature,
  columnSizingFeature,
  columnVisibilityFeature,
  createCoreRowModel,
  createTable,
  rowSelectionFeature,
  rowSortingFeature,
} from '@tanstack/svelte-table';

/**
 * Width of the row-number gutter (`td.row-num` / `th.row-num`). The gutter
 * renders inside the sticky start region but outside the table's pinning
 * model, so every pinned-column offset must include it or the first pinned
 * data column slides under the gutter.
 */
export const GUTTER_WIDTH = 44;

export interface GridColumn {
  name: string;
  type_name: string;
}

export interface SortState {
  /** Column INDEX as a string, not the display name — names can repeat. */
  id: string;
  desc: boolean;
}

export interface GridConfig {
  readonly columns: GridColumn[];
  readonly rows: unknown[][];
  readonly initialSorting: SortState[];
  readonly initialFilters: unknown[];
  /** Cap on ORDER BY keys. Beyond three the badge row stops being readable. */
  readonly maxSortKeys?: number;
  onSortingChange?: (sorting: SortState[]) => void;
}

/**
 * v9 features are opt-in modules. `stockFeatures` would restore v8's
 * everything-included behaviour at a bundle cost — deliberately not used.
 * columnSizing and columnResizing are SEPARATE in v9 and pinning-plus-resize
 * needs both.
 *
 * In v9.1.2 the feature flags AND row-model factories compose under ONE
 * `features` option passed to createTable (there are no `_features` /
 * `_rowModels` options in this version).
 */
export const GRID_FEATURES = {
  rowSortingFeature,
  columnFilteringFeature,
  columnSizingFeature,
  columnResizingFeature,
  columnVisibilityFeature,
  columnPinningFeature,
  rowSelectionFeature,
  cellSelectionFeature,
};

/**
 * v9 stores a selection as ordered rectangle operations keyed by flat row and
 * column ids (CellSelectionState). Structural type — the engine never imports
 * table-core types directly (spec §3.4 routes every type through LucentTable).
 */
interface CellSelectionRangeState {
  anchorRowId: string;
  anchorColumnId: string;
  focusRowId: string;
  focusColumnId: string;
}

export function createGridEngine(config: GridConfig) {
  let sorting = $state<SortState[]>([...config.initialSorting]);

  // v9.1.2's pinning state is `{ start, end }` (LTR/RTL-agnostic), not the
  // `{ left, right }` shape sketched when this task was planned — keys renamed
  // to match the shipped library.
  // v9's RowSelectionState is Record<string, true> — values are the literal
  // true, never false. Keeping this shape makes the table's state getter
  // assignment-compatible without casts.
  let rowSelection = $state<Record<string, true>>({});
  /** The row a shift-click extends FROM. Null until something is selected. */
  let selectionAnchor: number | null = null;

  let pinning = $state<{ start: string[]; end: string[] }>({
    start: [],
    end: [],
  });

  let cellSelection = $state<CellSelectionRangeState[]>([]);
  // Controlled sizing: without a state slice + change handler, a programmatic
  // setColumnSizing writes past the adapter's atom sync and getSize()/
  // getStart() read back null — which silently breaks pinned-offset math.
  // v9 shape: plain numbers per column id (NOT v8's {size} objects).
  let columnSizing = $state<Record<string, number>>({});

  /**
   * All three manual flags are pinned per spec D3 even though
   * rowPaginationFeature is deliberately NOT registered in phase 2 —
   * pagedStream owns display paging. v9 types gate `manualPagination`
   * behind that feature's registration, so the trio goes in via a spread:
   * the flag stays live at runtime (and asserted by tests) without
   * widening the type checking applied to every other option.
   */
  const MANUAL_FLAGS = {
    manualSorting: true,
    manualFiltering: true,
    manualPagination: true,
  } satisfies Record<string, boolean>;
  const columnDefs = $derived(
    config.columns.map((col, index) => ({
      // Index-based id: `SELECT a, a` yields duplicate names, and Table
      // requires unique ids. The display name lives in meta.
      id: String(index),
      accessorFn: (row: unknown[]) => row[index],
      meta: { name: col.name, typeName: col.type_name, index },
      enableSorting: true,
      enableMultiSort: true,
      enableColumnFilter: true,
      enableResizing: true,
      enableHiding: true,
      enablePinning: true,
    })),
  );

  const table = createTable({
    // Feature tuple stays a separate export so tests can assert on it; the
    // core row model joins it here. Core ONLY — adding sortedRowModel /
    // filteredRowModel / paginatedRowModel would silently operate on the
    // fetched slice while presenting it as the whole result. Spec D3.
    features: {
      ...GRID_FEATURES,
      coreRowModel: createCoreRowModel(),
    },
    ...MANUAL_FLAGS,
    // Legacy parity: a plain header click always started ascending, for every
    // column type. v9's default infers a desc-first cycle for numeric columns
    // (column_getFirstSortDir samples cell values), so pin it off. The column
    // menu still sets either direction explicitly.
    sortDescFirst: false,
    // Legacy parity: a header click flipped asc↔desc forever and never
    // cleared. v9's default enableSortingRemoval:true adds a third none state;
    // pin it off so the cycle stays two-state (the menu keeps Clear sort).
    enableSortingRemoval: false,
    // Multi-sort (phase ③): a plain click still replaces the single key with
    // the two-state cycle above; shift-click appends up to the cap. Keys are
    // sent to the backend in this order — badge number IS the ORDER BY
    // position.
    enableMultiSort: true,
    // Shift-click removes the last key too, so a user can walk a sort back
    // without clearing the whole thing.
    enableMultiRemove: true,
    maxMultiSortColCount: config.maxSortKeys ?? 3,
    isMultiSortEvent: (e: unknown) =>
      (e as { shiftKey?: boolean } | undefined)?.shiftKey === true,
    // The library default wipes cellSelection whenever the data identity
    // changes — and every fetch-more append replaces the accumulated array.
    // Lucent owns the lifecycle instead: emitChange clears explicitly when a
    // refetch actually reorders/replaces rows.
    autoResetCellSelection: false,
    // Rows are positional arrays with no natural key. Index over the
    // accumulated buffer is the absolute row number, which is also the
    // row-selection key: selection survives page changes by construction.
    getRowId: (_row: unknown[], index: number) => String(index),
    get columns() {
      return columnDefs;
    },
    get data() {
      return config.rows;
    },
    state: {
      get sorting() {
        return sorting;
      },
      get columnPinning() {
        return pinning;
      },
      get rowSelection() {
        return rowSelection;
      },
      get cellSelection() {
        return cellSelection;
      },
      get columnSizing() {
        return columnSizing;
      },
    },
    onSortingChange: (updater: unknown) => {
      sorting =
        typeof updater === 'function'
          ? (updater as (prev: SortState[]) => SortState[])(sorting)
          : (updater as SortState[]);
      config.onSortingChange?.(sorting);
    },
    onColumnSizingChange: (updater: unknown) => {
      columnSizing =
        typeof updater === 'function'
          ? (updater as (
              prev: Record<string, number>,
            ) => Record<string, number>)(columnSizing)
          : (updater as Record<string, number>);
    },
    onColumnPinningChange: (updater: unknown) => {
      // Every write re-normalizes the two keys: a partial or foreign-shaped
      // update must not strip `start`/`end` out from under the
      // getStart/getEnd* readers.
      const next =
        typeof updater === 'function'
          ? (
              updater as (p: { start: string[]; end: string[] }) => {
                start?: string[];
                end?: string[];
              }
 )({ ...pinning })
          : (updater as { start?: string[]; end?: string[] });
      pinning = {
        start: next?.start ?? [],
        end: next?.end ?? [],
      };
    },
    onRowSelectionChange: (updater: unknown) => {
      rowSelection =
        typeof updater === 'function'
          ? (updater as (p: typeof rowSelection) => typeof rowSelection)(rowSelection)
          : (updater as typeof rowSelection);
    },
    onCellSelectionChange: (updater: unknown) => {
      cellSelection =
        typeof updater === 'function'
          ? (updater as (
              p: typeof cellSelection,
            ) => typeof cellSelection)(cellSelection)
          : (updater as typeof cellSelection);
    },
  });

  /** 0-based position in the sort list, or -1 when this column is unsorted. */
  function sortIndexOf(columnId: string): number {
    return sorting.findIndex((s) => s.id === columnId);
  }

  /**
   * The current sort state in wire shape, in badge order. Interactive paths
   * cannot produce duplicate ids — toggle/multi-sort replace or cycle a
   * column's existing entry rather than appending a second one.
   */
  function sortingForWire(): { column: string; direction: 'asc' | 'desc' }[] {
    return sorting.map((s) => ({
      column: config.columns[Number(s.id)]?.name ?? s.id,
      direction: s.desc ? ('desc' as const) : ('asc' as const),
    }));
  }

  /** Absolute row indices, ascending. */
  function selectedRowIndices(): number[] {
    return Object.keys(rowSelection)
      .filter((k) => rowSelection[k])
      .map(Number)
      .sort((a, b) => a - b);
  }

  /**
   * Gutter click semantics, matching every file manager and spreadsheet:
   * plain click replaces, cmd/ctrl-click toggles, shift-click extends from
   * the last plain click.
   */
  function selectRow(
    absoluteIndex: number,
    opts: { extend: boolean; toggle: boolean },
  ) {
    if (opts.extend && selectionAnchor !== null) {
      const lo = Math.min(selectionAnchor, absoluteIndex);
      const hi = Math.max(selectionAnchor, absoluteIndex);
      const next: Record<string, true> = {};
      for (let i = lo; i <= hi; i += 1) next[String(i)] = true;
      rowSelection = next;
      return;
    }
    if (opts.toggle) {
      const next = { ...rowSelection };
      if (next[String(absoluteIndex)]) delete next[String(absoluteIndex)];
      else next[String(absoluteIndex)] = true;
      rowSelection = next;
      selectionAnchor = absoluteIndex;
      return;
    }
    rowSelection = { [String(absoluteIndex)]: true };
    selectionAnchor = absoluteIndex;
  }

  function clearRowSelection() {
    rowSelection = {};
    selectionAnchor = null;
  }

  /**
   * Union `indices` into the selection, keeping everything already selected
   * (including rows on other pages). Unlike selectRow's replace branch, this
   * is the primitive select-all affordances build on.
   */
  function selectRows(indices: number[]) {
    const next = { ...rowSelection };
    for (const i of indices) next[String(i)] = true;
    rowSelection = next;
  }

  /**
   * `false` unpins. Pinning is view state only — it never reaches the wire.
   * The UI speaks left/right; v9's pin API speaks the LTR/RTL-logical
   * start/end and treats any other string as unpin, so this seam maps.
   */
  function pinColumn(columnId: string, side: 'left' | 'right' | false) {
    table
      .getColumn(columnId)
      ?.pin(side === false ? false : side === 'left' ? 'start' : 'end');
  }

  // ---- Cell-range selection (phase ③). Wrappers over v9's
  // cellSelectionFeature statics — wrapped, never re-implemented.
  // Reconciled against the SHIPPED types
  // (table-core/dist/features/cell-selection/cellSelectionFeature.types.d.ts),
  // which differ from the shapes sketched when this task was planned:
  //   setFocusedCell(rowId, columnId) is positional, not object-taking;
  //   selectCellRange takes {anchorRowId, anchorColumnId, focusRowId,
  //   focusColumnId} corners, not {start, end}; extendCellSelection takes a
  //   DIRECTION, not a target cell; moveCellSelection takes no extend flag.

  /**
   * Row-major values for the selected rectangles, ready to serialise. The
   * library returns one grid per region ([region][row][col]); replace-mode
   * selections have exactly one region, and multi-region selections (future
   * cmd-click additive ranges) concatenate their grids row-wise.
   */
  function selectedCellRangesData(): unknown[][] {
    return table.getSelectedCellRangesData().flat();
  }

  function startCellSelection(target: { rowIndex: number; columnId: string }) {
    const rowId = String(target.rowIndex);
    table.setFocusedCell(rowId, target.columnId);
    table.selectCellRange({
      anchorRowId: rowId,
      anchorColumnId: target.columnId,
      focusRowId: rowId,
      focusColumnId: target.columnId,
    });
  }

  function extendCellSelection(target: { rowIndex: number; columnId: string }) {
    // v9 has no target-based extend — reproduce drag semantics by re-selecting
    // from the current anchor to the target. The anchor comes from THIS
    // engine's synchronous state, not table.getFocusedCell(): the svelte
    // adapter syncs table atoms on the microtask queue, so the library's
    // atom-backed getter can lag one turn behind a just-made selection.
    // With nothing selected yet, this degrades to starting at the target.
    const active = cellSelection[cellSelection.length - 1];
    table.selectCellRange({
      anchorRowId: active?.anchorRowId ?? String(target.rowIndex),
      anchorColumnId: active?.anchorColumnId ?? target.columnId,
      focusRowId: String(target.rowIndex),
      focusColumnId: target.columnId,
    });
  }

  function moveCellSelection(
    direction: 'up' | 'down' | 'left' | 'right',
    extend: boolean,
  ) {
    if (extend) {
      // Shift-arrow keeps the anchor and grows the range one step.
      table.extendCellSelection(direction);
      return;
    }
    // Plain arrow collapses the selection onto the neighbouring cell.
    table.moveCellSelection(direction);
  }

  function selectAllCells() {
    table.selectAllCells();
  }

  function clearCellSelection() {
    // Explicit empty reset regardless of initial state. Note the library also
    // auto-resets when `data` changes (autoResetCellSelection defaults true):
    // a refetch clears stale ranges for free.
    table.resetCellSelection(true);
  }

  return {
    table,
    get sorting() {
      return sorting;
    },
    get pinning() {
      return pinning;
    },
    get rowSelection() {
      return rowSelection;
    },
    selectedRowIndices,
    selectRow,
    clearRowSelection,
    selectRows,
    pinColumn,
    sortIndexOf,
    sortingForWire,
    get cellSelection() {
      return cellSelection;
    },
    selectedCellRangesData,
    startCellSelection,
    extendCellSelection,
    moveCellSelection,
    selectAllCells,
    clearCellSelection,
  };
}

/** The one table type every component prop uses. See spec §3.4. */
export type LucentTable = ReturnType<typeof createGridEngine>['table'];
