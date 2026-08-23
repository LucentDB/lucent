<script lang="ts">
  // createGridEngine opens $effect scopes, so it must run during component
  // init. This harness exists so a render test can hand GridBody a real
  // table instance — same pattern as HeaderHarness.
  import { createGridEngine } from './engine.svelte.ts';
  import GridBody from './GridBody.svelte';

  let {
    columns,
    rows,
    pageRows,
    pageOffset = 0,
    columnWidths = {},
    selectedRows = new Set(),
    onSelectRow = () => {},
    onCellContextMenu = () => {},
    /** Column ids to pin left before first render — drives group layout. */
    pinLeft = [],
    /** Column ids to pin right before first render — drives group layout. */
    pinRight = [],
  }: {
    columns?: { name: string; type_name: string }[];
    rows?: unknown[][];
    pageRows?: unknown[][];
    pageOffset?: number;
    columnWidths?: Record<number, number>;
    selectedRows?: Set<number>;
    onSelectRow?: (
      absoluteIndex: number,
      opts: { extend: boolean; toggle: boolean },
    ) => void;
    onCellContextMenu?: (e: MouseEvent, columnIndex: number, value: unknown) => void;
    pinLeft?: string[];
    pinRight?: string[];
  } = $props();

  const engine = createGridEngine({
    get columns() { return columns ?? []; },
    get rows() { return rows ?? []; },
    get initialSorting() { return []; },
    get initialFilters() { return []; },
  });
  // Deliberately read once at init: the harness pins before first render so
  // group layout is settled when the test's first assertions run.
  for (const id of pinLeft) engine.pinColumn(id, 'left');
  for (const id of pinRight) engine.pinColumn(id, 'right');
</script>

<table>
  <GridBody
    table={engine.table}
    {pageRows}
    {pageOffset}
    {columnWidths}
    {selectedRows}
    {onSelectRow}
    {onCellContextMenu}
  />
</table>
