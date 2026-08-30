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
    sortIndexOf = () => -1,
    sortDirectionOf = () => false,
    onToggleSort = () => {},
    onOpenMenu = () => {},
    onResizeStart = () => {},
    onResizeKeydown = () => {},
    onToggleSelectAllPage = () => {},
    allPageSelected = false,
    /** Column ids to pin left before first render — drives group layout. */
    pinLeft = [],
    /** Column ids to pin right before first render — drives group layout. */
    pinRight = [],
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
  for (const id of pinLeft) engine.pinColumn(id, 'left');
  for (const id of pinRight) engine.pinColumn(id, 'right');

  /** Post-render sizing hook: tests drive the real table API from outside. */
  export function resizeTo(id, size) {
    engine.table.setColumnSizing({ [id]: size });
  }
</script>

<table>
  <GridHeader
    table={engine.table}
    {columnWidths}
    {sortIndexOf}
    {sortDirectionOf}
    {onToggleSort}
    {onOpenMenu}
    {onResizeStart}
    {onResizeKeydown}
    {onToggleSelectAllPage}
    {allPageSelected}
  />
</table>
