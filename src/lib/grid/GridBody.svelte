<script>
  // formatCell stays here in Lucent's own markup rather than moving into
  // column cell renderers: those need FlexRender in the body, which would
  // break the rule that only engine.svelte.ts touches the adapter. Spec §3.3.
  import { formatCell, cellClass } from './format.js';
  import { edgesOf } from './selection.js';

  let {
    table,
    pageRows = [],
    pageOffset = 0,
    columnWidths = {},
    checkedRows = new Set(),
    onToggleCheck,
    onCellContextMenu,
    /** Normalised selection rectangle, or null. See selection.js. */
    selBounds = null,
    onCellMouseDown = null,
    onCellMouseEnter = null,
  } = $props();

  // Paging is manual (spec D3), so the table's row model holds the whole
  // accumulated buffer while the body renders one page slice of it. Column
  // order still comes from the table.
  const leafColumns = $derived(table.getVisibleLeafColumns());

  // A lone cell reads as a cursor and fills solid, the way a native table
  // view draws its selected cell. A range is a region instead: a wash plus a
  // single marquee around the outside, so the individual cells inside it stay
  // readable.
  const single = $derived(
    selBounds !== null &&
      selBounds.r0 === selBounds.r1 &&
      selBounds.c0 === selBounds.c1,
  );

  function widthOf(index) {
    return columnWidths[index] || 150;
  }

  /**
   * The marquee, as inset shadows on whichever sides face outward. Built
   * inline rather than as classes because a cell can be on any combination of
   * the four edges, and CSS rules would each overwrite the others' shadow.
   */
  function edgeShadow(row, col) {
    if (single) return '';
    const e = edgesOf(selBounds, row, col);
    const parts = [];
    if (e.top) parts.push('inset 0 1px 0 0 var(--accent)');
    if (e.bottom) parts.push('inset 0 -1px 0 0 var(--accent)');
    if (e.left) parts.push('inset 1px 0 0 0 var(--accent)');
    if (e.right) parts.push('inset -1px 0 0 0 var(--accent)');
    return parts.length ? `box-shadow: ${parts.join(', ')};` : '';
  }

  function inSelection(row, col) {
    if (selBounds === null) return false;
    return (
      row >= selBounds.r0 &&
      row <= selBounds.r1 &&
      col >= selBounds.c0 &&
      col <= selBounds.c1
    );
  }
</script>

<tbody>
  {#each pageRows as row, i}
    {@const absolute = pageOffset + i}
    <tr class:even={absolute % 2 === 0}>
      <td class="row-num">
        <input
          type="checkbox"
          onchange={() => onToggleCheck(absolute)}
          checked={checkedRows.has(absolute)}
        />
      </td>
      {#each leafColumns as column, col (column.id)}
        {@const index = column.columnDef.meta?.index ?? 0}
        {@const cell = row[index]}
        {@const selected = inSelection(i, col)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <td
          class={cellClass(cell)}
          class:sel={selected}
          class:sel-solid={selected && single}
          aria-selected={selected}
          style="width: {widthOf(index)}px; min-width: 80px; {edgeShadow(
            i,
            col,
          )}"
          onmousedown={(e) => onCellMouseDown?.(e, i, col)}
          onmouseenter={() => onCellMouseEnter?.(i, col)}
          oncontextmenu={(e) => onCellContextMenu(e, index, cell)}
        >
          <span class="cell-content">{formatCell(cell)}</span>
        </td>
      {/each}
    </tr>
  {/each}
</tbody>

<style>
  /* Cells are objects, not text. Without this the browser drags a ragged
     character selection across cell boundaries, which is the clearest single
     tell that a table is a web page. Selection is owned by selection.js;
     Cmd+C and the cell menu copy the underlying value, so nothing is lost. */
  td {
    user-select: none;
    -webkit-user-select: none;
    cursor: default;
  }

  /* Alternating rows, at the barely-there contrast a native table view uses:
     enough to follow a row across a wide result, not enough to stripe. */
  tr.even td {
    background: var(--bg-subtle);
  }
  tr:hover td {
    background: var(--bg-hover);
  }

  td {
    /* ~24px rows. The old 8px/12px padding gave 36px, which wasted a third of
       the vertical space in a tool whose whole job is showing many rows. */
    padding: 3px 8px;
    font-size: var(--text-sm);
    height: var(--grid-row-h);
    border-bottom: 1px solid var(--grid-line);
    border-right: 1px solid var(--grid-line);
    white-space: nowrap;
  }
  td:last-child {
    border-right: none;
  }
  td.row-num {
    text-align: center;
    padding: 3px 4px;
    width: 34px;
    border-right: 1px solid var(--grid-line);
  }
  td.row-num input {
    cursor: pointer;
  }

  /* ─── Selection ──────────────────────────────────────────────────────
     Precedence matters: these follow the zebra and hover rules so a
     selected cell stays selected-looking under the pointer. */
  tr td.sel,
  tr:hover td.sel,
  tr.even td.sel {
    background: var(--accent-soft);
  }
  /* The cursor cell: solid fill, as AppKit draws it. */
  tr td.sel-solid,
  tr:hover td.sel-solid,
  tr.even td.sel-solid {
    background: var(--accent);
    color: var(--accent-foreground);
  }
  td.sel-solid.cell-null {
    color: var(--accent-foreground);
    opacity: 0.7;
  }

  td.cell-null {
    color: var(--text-muted);
    font-style: italic;
  }
  td.cell-number {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  /* Booleans are data, not status. The tinted pills they used to render as
     claimed a significance `false` does not have. */
  td.cell-bool {
    text-align: left;
    color: var(--text-secondary);
  }
  td.cell-bool.sel-solid {
    color: var(--accent-foreground);
  }

  /* Cell content ellipsis */
  .cell-content {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Checkbox styling (body side; the header copy stays in ResultsGrid
     until Task 8 rewrites it) */
  td input[type='checkbox'] {
    width: 13px;
    height: 13px;
    accent-color: var(--accent);
    cursor: pointer;
  }
</style>
