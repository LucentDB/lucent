<script>
  // createGridEngine opens $effect scopes, so it must run during component
  // init. This harness exists so a render test can hand GridHeader a real
  // table instance. See spec §3.2.
  import { createGridEngine } from './engine.svelte.ts';
  import GridHeader from './GridHeader.svelte';

  let {
    columns,
    rows,
    columnWidths = {},
    sortIndicatorFor = () => '',
    onToggleSort = () => {},
    onOpenMenu = () => {},
    onResizeStart = () => {},
    onResizeKeydown = () => {},
    onToggleCheckAll = () => {},
    allChecked = false,
  } = $props();

  const engine = createGridEngine({
    get columns() {
      return columns;
    },
    get rows() {
      return rows;
    },
    get initialSorting() {
      return [];
    },
    get initialFilters() {
      return [];
    },
  });
</script>

<table>
  <GridHeader
    table={engine.table}
    {columnWidths}
    {sortIndicatorFor}
    {onToggleSort}
    {onOpenMenu}
    {onResizeStart}
    {onResizeKeydown}
    {onToggleCheckAll}
    {allChecked}
  />
</table>
