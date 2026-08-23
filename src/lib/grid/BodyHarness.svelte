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
    checkedRows = new Set(),
    onToggleCheck = () => {},
    onCellContextMenu = () => {},
  }: {
    columns?: { name: string; type_name: string }[];
    rows?: unknown[][];
    pageRows?: unknown[][];
    pageOffset?: number;
    columnWidths?: Record<number, number>;
    checkedRows?: Set<number>;
    onToggleCheck?: (absoluteIndex: number) => void;
    onCellContextMenu?: (e: MouseEvent, columnIndex: number, value: unknown) => void;
  } = $props();

  const engine = createGridEngine({
    get columns() { return columns ?? []; },
    get rows() { return rows ?? []; },
    get initialSorting() { return []; },
    get initialFilters() { return []; },
  });
</script>

<table>
  <GridBody
    table={engine.table}
    {pageRows}
    {pageOffset}
    {columnWidths}
    {checkedRows}
    {onToggleCheck}
    {onCellContextMenu}
  />
</table>
