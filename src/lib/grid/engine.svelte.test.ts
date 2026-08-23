import { describe, it, expect, vi } from 'vitest';
import { flushSync } from 'svelte';
import { createGridEngine, GRID_FEATURES } from './engine.svelte.ts';

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

  it('does not yet include cellSelectionFeature — phase 3 adds it with its UI', () => {
    expect(Object.keys(GRID_FEATURES)).not.toContain('cellSelectionFeature');
  });
});
