import { describe, it, expect } from 'vitest';
import { flushSync } from 'svelte';
import {
  createGridEngine,
  computeGutterWidth,
  GRID_FEATURES,
} from './engine.svelte.ts';

const COLUMNS = [
  { name: 'id', type_name: 'int4' },
  { name: 'email', type_name: 'text' },
];

const ROWS: unknown[][] = [
  [1, 'a@x.com'],
  [2, 'b@x.com'],
];

/**
 * The engine opens $effect scopes, so every case runs inside $effect.root and
 * disposes at the end — the same constraint ResultsGrid satisfies by being a
 * component. See spec §3.2.
 *
 * Deviation from the brief's harness (same treatment pagedStream tests needed):
 * backing state is `$state` and mount-time effects are settled with
 * `flushSync()` — plain-object getter mutations never invalidate effects, so
 * the reactivity cases below would fail inertly, and un-settled mount effects
 * would race the first interaction. Assertions stay verbatim.
 */
function harness(init?: {
  columns?: typeof COLUMNS;
  rows?: unknown[][];
  onSortingChange?: (s: unknown) => void;
}) {
  const state = $state({
    columns: init?.columns ?? COLUMNS,
    rows: init?.rows ?? ROWS,
    initialSorting: [] as { id: string; desc: boolean }[],
    initialFilters: [] as unknown[],
  });
  let engine!: ReturnType<typeof createGridEngine>;
  const dispose = $effect.root(() => {
    engine = createGridEngine({
      get columns() {
        return state.columns;
      },
      get rows() {
        return state.rows;
      },
      get initialSorting() {
        return state.initialSorting;
      },
      get initialFilters() {
        return state.initialFilters;
      },
      onSortingChange: init?.onSortingChange,
    });
    flushSync();
  });
  const flush = () => new Promise((r) => setTimeout(r, 0));
  return { state, engine: () => engine, flush, dispose };
}

describe('manual mode — the D3 correctness guards', () => {
  it('registers ONLY the core row model', () => {
    const h = harness();
    // v9.1.2 composes feature flags AND row-model factories under one
    // `features` option (the brief's `_features`/`_rowModels` keys do not
    // exist in this version). Same guarantee, real surface:
    const f = h.engine().table.options.features ?? {};
    for (const required of [
      'rowSortingFeature',
      'columnFilteringFeature',
      'columnSizingFeature',
      'columnResizingFeature',
      'columnVisibilityFeature',
      'columnPinningFeature',
      'rowSelectionFeature',
      // Injected by the svelte adapter itself, not chosen by us.
      'coreReactivityFeature',
      // Core ONLY — registering Sorted/Filtered/Paginated would sort or filter
      // the fetched slice while presenting it as the whole result. See spec D3.
      'coreRowModel',
    ]) {
      expect(f).toHaveProperty(required);
    }
    for (const banned of [
      'sortedRowModel',
      'filteredRowModel',
      'paginatedRowModel',
    ]) {
      expect(f).not.toHaveProperty(banned);
    }
    // And the runtime cache holds real factories only where core registered
    // one — unregistered kinds appear as undefined-valued keys once their
    // getter is consulted, so filter to defined values.
    h.engine().table.getRowModel();
    const cached = Reflect.get(h.engine().table, '_rowModels') as
      unknown | undefined;
    const materialized = Object.entries(
      (cached ?? {}) as Record<string, unknown>,
    )
      .filter(([, factory]) => factory !== undefined)
      .map(([key]) => key);
    expect(materialized).toEqual(['coreRowModel']);
    h.dispose();
  });

  it('sets all three manual flags', () => {
    const h = harness();
    const o = h.engine().table.options;
    expect(o.manualSorting).toBe(true);
    expect(o.manualFiltering).toBe(true);
    // Typed out of v9's option surface until rowPaginationFeature registers
    // (phase 3); the engine pins it at runtime per spec D3.
    expect(Reflect.get(o, 'manualPagination')).toBe(true);
    h.dispose();
  });

  it('does not reorder rows locally when sorting state changes', async () => {
    const h = harness();
    h.engine().table.setSorting([{ id: '0', desc: true }]);
    await h.flush();
    const rendered = h
      .engine()
      .table.getRowModel()
      .rows.map((r) => r.original);
    // Backend order is authoritative; the row model must be untouched.
    expect(rendered).toEqual(ROWS);
    h.dispose();
  });

  it('does not reduce rows locally when filter state changes', async () => {
    const h = harness();
    h.engine().table.setColumnFilters([{ id: '1', value: 'nomatch' }]);
    await h.flush();
    expect(h.engine().table.getRowModel().rows).toHaveLength(ROWS.length);
    h.dispose();
  });

  it('cycles header clicks asc↔desc forever — never a third cleared state', async () => {
    // Legacy toggleSort flipped asc↔desc indefinitely; v9's default
    // enableSortingRemoval:true would land on none after the second toggle.
    // The engine pins enableSortingRemoval:false, so this pins the two-state
    // cycle ResultsGrid's header has always had.
    const h = harness();
    h.engine().table.setSorting([{ id: '0', desc: false }]);
    await h.flush();

    h.engine().table.getColumn('0')!.toggleSorting();
    await h.flush();
    expect(h.engine().table.atoms.sorting.get()).toEqual([
      { id: '0', desc: true },
    ]);

    h.engine().table.getColumn('0')!.toggleSorting();
    await h.flush();
    // Back to ascending — NOT removed/undefined.
    expect(h.engine().table.atoms.sorting.get()).toEqual([
      { id: '0', desc: false },
    ]);
    h.dispose();
  });
});

describe('column defs', () => {
  it('ids columns by index so duplicate names stay distinct', () => {
    // SELECT a, a is legal SQL and yields two columns named 'a'.
    const h = harness({
      columns: [
        { name: 'a', type_name: 'int4' },
        { name: 'a', type_name: 'text' },
      ],
    });
    const ids = h
      .engine()
      .table.getAllLeafColumns()
      .map((c) => c.id);
    expect(ids).toEqual(['0', '1']);
    h.dispose();
  });

  it('carries the display name and type in column meta', () => {
    const h = harness();
    const col = h.engine().table.getColumn('1');
    expect(col?.columnDef.meta).toMatchObject({
      name: 'email',
      typeName: 'text',
    });
    h.dispose();
  });

  it('reads array rows by position', () => {
    const h = harness();
    const first = h.engine().table.getRowModel().rows[0];
    expect(first.getValue('1')).toBe('a@x.com');
    h.dispose();
  });
});

describe('reactivity across the getter boundary', () => {
  it('rebuilds the row model when new rows accumulate', async () => {
    const h = harness();
    expect(h.engine().table.getRowModel().rows).toHaveLength(2);

    // The spec §3.2 failure mode: a value-passed option would leave this at 2.
    h.state.rows = [...ROWS, [3, 'c@x.com']];
    await h.flush();

    expect(h.engine().table.getRowModel().rows).toHaveLength(3);
    h.dispose();
  });

  it('rebuilds columns when the result shape changes', async () => {
    const h = harness();
    h.state.columns = [{ name: 'only', type_name: 'text' }];
    h.state.rows = [['x']];
    await h.flush();
    expect(h.engine().table.getAllLeafColumns()).toHaveLength(1);
    h.dispose();
  });
});

describe('multi-sort', () => {
  it('reports no sort index for an unsorted column', () => {
    const h = harness();
    expect(h.engine().sortIndexOf('0')).toBe(-1);
    h.dispose();
  });

  it('reports 0-based positions matching the badge numbers', async () => {
    const h = harness();
    h.engine().table.setSorting([
      { id: '1', desc: false },
      { id: '0', desc: true },
    ]);
    await h.flush();
    expect(h.engine().sortIndexOf('1')).toBe(0);
    expect(h.engine().sortIndexOf('0')).toBe(1);
    h.dispose();
  });

  it('treats a shift-click as a multi-sort event', () => {
    const h = harness();
    const isMulti = h.engine().table.options.isMultiSortEvent;
    expect(isMulti?.({ shiftKey: true } as never)).toBe(true);
    expect(isMulti?.({ shiftKey: false } as never)).toBe(false);
    h.dispose();
  });

  it('caps the number of sort keys', async () => {
    const h = harness({
      columns: [
        { name: 'a', type_name: 'int4' },
        { name: 'b', type_name: 'int4' },
        { name: 'c', type_name: 'int4' },
        { name: 'd', type_name: 'int4' },
      ],
      rows: [[1, 2, 3, 4]],
    });
    expect(h.engine().table.options.maxMultiSortColCount).toBe(3);
    h.dispose();
  });

  it('enforces the cap behaviorally: a fourth shift-click is refused', async () => {
    const h = harness({
      columns: [
        { name: 'a', type_name: 'int4' },
        { name: 'b', type_name: 'int4' },
        { name: 'c', type_name: 'int4' },
        { name: 'd', type_name: 'int4' },
      ],
      rows: [[1, 2, 3, 4]],
    });
    // The same path a shift-click takes in the header: getToggleSortingHandler
    // applies isMultiSortEvent and appends a key — until the cap.
    for (const id of ['0', '1', '2', '3']) {
      h.engine().table.getColumn(id)?.getToggleSortingHandler()?.({
        shiftKey: true,
      });
    }
    await h.flush();
    expect(h.engine().sorting).toHaveLength(3);
    h.dispose();
  });

  it('emits every key to the wire, in badge order', async () => {
    const h = harness();
    h.engine().table.setSorting([
      { id: '1', desc: false },
      { id: '0', desc: true },
    ]);
    await h.flush();
    expect(h.engine().sortingForWire()).toEqual([
      { column: 'email', direction: 'asc' },
      { column: 'id', direction: 'desc' },
    ]);
    h.dispose();
  });

  it('still does not reorder rows locally with two keys set', async () => {
    const h = harness();
    h.engine().table.setSorting([
      { id: '1', desc: false },
      { id: '0', desc: true },
    ]);
    await h.flush();
    // Manual mode holds regardless of how many keys are set. Spec D3.
    expect(
      h
        .engine()
        .table.getRowModel()
        .rows.map((r) => r.original),
    ).toEqual(ROWS);
    h.dispose();
  });
});

describe('GRID_FEATURES', () => {
  it('includes every feature the grid renders against', () => {
    const names = Object.keys(GRID_FEATURES);
    for (const required of [
      'rowSortingFeature',
      'columnFilteringFeature',
      'columnSizingFeature',
      'columnResizingFeature',
      'columnVisibilityFeature',
      'columnPinningFeature',
      'rowSelectionFeature',
    ]) {
      expect(names).toContain(required);
    }
  });

  it('includes cellSelectionFeature', () => {
    expect(Object.keys(GRID_FEATURES)).toContain('cellSelectionFeature');
  });
});

describe('column pinning', () => {
  it('starts with nothing pinned', () => {
    const h = harness();
    expect(h.engine().table.getStartVisibleLeafColumns()).toHaveLength(0);
    h.dispose();
  });

  it('moves a pinned column into the start group', async () => {
    const h = harness();
    h.engine().pinColumn('0', 'left');
    await h.flush();
    expect(
      h
        .engine()
        .table.getStartVisibleLeafColumns()
        .map((c) => c.id),
    ).toEqual(['0']);
    expect(
      h
        .engine()
        .table.getCenterVisibleLeafColumns()
        .map((c) => c.id),
    ).toEqual(['1']);
    h.dispose();
  });

  it('unpins back into the centre group', async () => {
    const h = harness();
    h.engine().pinColumn('0', 'left');
    await h.flush();
    h.engine().pinColumn('0', false);
    await h.flush();
    expect(h.engine().table.getStartVisibleLeafColumns()).toHaveLength(0);
    expect(h.engine().table.getCenterVisibleLeafColumns()).toHaveLength(2);
    h.dispose();
  });

  it('reports whether a column is pinned, for the menu state', async () => {
    const h = harness();
    h.engine().pinColumn('1', 'right');
    await h.flush();
    expect(h.engine().table.getColumn('1')?.getIsPinned()).toBe(
      'end',
    ); /* library's logical region for right */
    h.dispose();
  });

  it('keeps pinning independent of sorting', async () => {
    const h = harness();
    h.engine().pinColumn('0', 'left');
    h.engine().table.setSorting([{ id: '0', desc: true }]);
    await h.flush();
    expect(h.engine().table.getColumn('0')?.getIsPinned()).toBe(
      'start',
    ); /* library's logical region for left */
    expect(h.engine().sortingForWire()).toEqual([
      { column: 'id', direction: 'desc' },
    ]);
    h.dispose();
  });
});

describe('cell-range selection', () => {
  it('starts with no cells selected', () => {
    const h = harness();
    expect(h.engine().selectedCellRangesData()).toEqual([]);
    h.dispose();
  });

  it('selects a single cell', async () => {
    const h = harness();
    h.engine().startCellSelection({ rowIndex: 0, columnId: '1' });
    await h.flush();
    expect(h.engine().selectedCellRangesData()).toEqual([['a@x.com']]);
    h.dispose();
  });

  it('extends into a rectangle, row-major', async () => {
    const h = harness();
    h.engine().startCellSelection({ rowIndex: 0, columnId: '0' });
    h.engine().extendCellSelection({ rowIndex: 1, columnId: '1' });
    await h.flush();
    expect(h.engine().selectedCellRangesData()).toEqual([
      [1, 'a@x.com'],
      [2, 'b@x.com'],
    ]);
    h.dispose();
  });

  it('extends backwards to the same rectangle', async () => {
    const h = harness();
    h.engine().startCellSelection({ rowIndex: 1, columnId: '1' });
    h.engine().extendCellSelection({ rowIndex: 0, columnId: '0' });
    await h.flush();
    expect(h.engine().selectedCellRangesData()).toEqual([
      [1, 'a@x.com'],
      [2, 'b@x.com'],
    ]);
    h.dispose();
  });

  it('selects every cell', async () => {
    const h = harness();
    h.engine().selectAllCells();
    await h.flush();
    expect(h.engine().selectedCellRangesData()).toHaveLength(2);
    h.dispose();
  });

  it('clears', async () => {
    const h = harness();
    h.engine().startCellSelection({ rowIndex: 0, columnId: '0' });
    await h.flush();
    h.engine().clearCellSelection();
    await h.flush();
    expect(h.engine().selectedCellRangesData()).toEqual([]);
    h.dispose();
  });

  it('moves the focused cell with arrow semantics', async () => {
    const h = harness();
    h.engine().startCellSelection({ rowIndex: 0, columnId: '0' });
    // The library's move/extend statics read the table atom, which the svelte
    // adapter syncs on the microtask queue; a keypress is its own task, so
    // real usage always sees settled atoms. The flush mirrors that timing.
    await h.flush();
    h.engine().moveCellSelection('right', false);
    await h.flush();
    expect(h.engine().selectedCellRangesData()).toEqual([['a@x.com']]);
    h.dispose();
  });

  it('shift-arrow grows the range instead of moving it', async () => {
    const h = harness();
    h.engine().startCellSelection({ rowIndex: 0, columnId: '0' });
    await h.flush();
    h.engine().moveCellSelection('right', true);
    await h.flush();
    expect(h.engine().selectedCellRangesData()).toEqual([[1, 'a@x.com']]);
    h.dispose();
  });

  it('leaves row selection alone', async () => {
    const h = harness();
    h.engine().selectRow(0, { extend: false, toggle: false });
    h.engine().startCellSelection({ rowIndex: 1, columnId: '0' });
    await h.flush();
    // The two selection models are independent: cells for copy, rows for
    // row-scoped actions. Spec §4.3.
    expect(h.engine().selectedRowIndices()).toEqual([0]);
    h.dispose();
  });
});

describe('row selection', () => {
  it('starts with nothing selected', () => {
    const h = harness();
    expect(h.engine().selectedRowIndices()).toEqual([]);
    h.dispose();
  });

  it('selects one row, replacing any previous selection', async () => {
    const h = harness({
      rows: [
        [1, 'a'],
        [2, 'b'],
        [3, 'c'],
      ],
    });
    h.engine().selectRow(0, { extend: false, toggle: false });
    await h.flush();
    h.engine().selectRow(2, { extend: false, toggle: false });
    await h.flush();
    expect(h.engine().selectedRowIndices()).toEqual([2]);
    h.dispose();
  });

  it('cmd-click toggles a row into the selection', async () => {
    const h = harness({
      rows: [
        [1, 'a'],
        [2, 'b'],
        [3, 'c'],
      ],
    });
    h.engine().selectRow(0, { extend: false, toggle: false });
    h.engine().selectRow(2, { extend: false, toggle: true });
    await h.flush();
    expect(h.engine().selectedRowIndices()).toEqual([0, 2]);
    h.dispose();
  });

  it('shift-click extends from the anchor to the clicked row', async () => {
    const h = harness({
      rows: [
        [1, 'a'],
        [2, 'b'],
        [3, 'c'],
        [4, 'd'],
      ],
    });
    h.engine().selectRow(1, { extend: false, toggle: false });
    h.engine().selectRow(3, { extend: true, toggle: false });
    await h.flush();
    expect(h.engine().selectedRowIndices()).toEqual([1, 2, 3]);
    h.dispose();
  });

  it('extends backwards too', async () => {
    const h = harness({
      rows: [
        [1, 'a'],
        [2, 'b'],
        [3, 'c'],
        [4, 'd'],
      ],
    });
    h.engine().selectRow(3, { extend: false, toggle: false });
    h.engine().selectRow(1, { extend: true, toggle: false });
    await h.flush();
    expect(h.engine().selectedRowIndices()).toEqual([1, 2, 3]);
    h.dispose();
  });

  it('keeps selection across a page change, since indices are absolute', async () => {
    const h = harness({ rows: Array.from({ length: 400 }, (_, i) => [i]) });
    h.engine().selectRow(250, { extend: false, toggle: false });
    await h.flush();
    // Paging is a display concern; the selection lives on the table.
    expect(h.engine().selectedRowIndices()).toEqual([250]);
    h.dispose();
  });
});

describe('computeGutterWidth', () => {
  it('returns base 34px for 1 and 2-digit row counts', () => {
    expect(computeGutterWidth(0)).toBe(34);
    expect(computeGutterWidth(1)).toBe(34);
    expect(computeGutterWidth(99)).toBe(34);
  });

  it('scales cleanly for 3, 4, 5, and 6-digit counts so numbers never overflow', () => {
    expect(computeGutterWidth(100)).toBe(42);
    expect(computeGutterWidth(999)).toBe(42);
    expect(computeGutterWidth(2607)).toBe(50);
    expect(computeGutterWidth(10000)).toBe(58);
    expect(computeGutterWidth(100000)).toBe(66);
  });
});
