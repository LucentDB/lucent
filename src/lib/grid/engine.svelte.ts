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

export interface GridColumn {
  name: string;
  type_name: string;
}

export interface SortState {
  /** Column INDEX as a string, not the display name — names can repeat. */
  id: string;
  desc: boolean;
}

export interface WireSort {
  column: string;
  direction: 'asc' | 'desc';
}

export interface GridConfig {
  readonly columns: GridColumn[];
  readonly rows: unknown[][];
  readonly initialSorting: SortState[];
  readonly initialFilters: unknown[];
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
};

export function createGridEngine(config: GridConfig) {
  let sorting = $state<SortState[]>([...config.initialSorting]);

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
    // Rows are positional arrays with no natural key. Index over the
    // accumulated buffer is the absolute row number, matching the selection
    // semantics the old checkedRows Set used.
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
    },
    onSortingChange: (updater: unknown) => {
      sorting =
        typeof updater === 'function'
          ? (updater as (prev: SortState[]) => SortState[])(sorting)
          : (updater as SortState[]);
      config.onSortingChange?.(sorting);
    },
  });

  /** Table sorting state → the IPC `sort` shape, ids resolved back to names. */
  function sortingForWire(): WireSort[] {
    return sorting.map((s) => ({
      column: config.columns[Number(s.id)]?.name ?? s.id,
      direction: s.desc ? 'desc' : 'asc',
    }));
  }

  return {
    table,
    get sorting() {
      return sorting;
    },
    sortingForWire,
  };
}

/** The one table type every component prop uses. See spec §3.4. */
export type LucentTable = ReturnType<typeof createGridEngine>['table'];
