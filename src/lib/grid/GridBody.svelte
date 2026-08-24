<script>
  // formatCell stays here in Lucent's own markup rather than moving into
  // column cell renderers: those need FlexRender in the body, which would
  // break the rule that only engine.svelte.ts touches the adapter. Spec §3.3.
  import { formatCell, cellClass } from './format.js';
  import { GUTTER_WIDTH } from './engine.svelte.ts';
  import GridGutter from './GridGutter.svelte';

  let {
    table,
    pageRows = [],
    pageOffset = 0,
    columnWidths = {},
    selectedRows = new Set(),
    onSelectRow,
    onCellContextMenu,
    onCellMouseDown = () => {},
    onCellMouseEnter = () => {},
  } = $props();

  // Paging is manual (spec D3), so the table's row model holds the whole
  // accumulated buffer while the body renders one page slice of it. Column
  // order still comes from the table.
  const startCount = $derived(table.getStartVisibleLeafColumns().length);

  function widthOf(index) {
    return columnWidths[index] || 150;
  }
</script>

<tbody>
  {#each pageRows as row, i}
    {@const absolute = pageOffset + i}
    {@const tableRow = table.getRowModel().rows[absolute]}
    <tr class:even={absolute % 2 === 0}>
      <td class="row-num cell-start" style="left: 0;">
        <GridGutter
          rowNumber={absolute + 1}
          selected={selectedRows.has(absolute)}
          onSelect={(e) =>
            onSelectRow(absolute, {
              extend: e.shiftKey,
              toggle: e.metaKey || e.ctrlKey,
            })}
        />
      </td>
      {#each tableRow?.getStartVisibleCells() ?? [] as cell, ci (cell.id)}
        {@render bodyCell(cell, row, ci === startCount - 1 ? 'cell-start pinned-edge' : 'cell-start', absolute)}
      {/each}
      {#each tableRow?.getCenterVisibleCells() ?? [] as cell (cell.id)}
        {@render bodyCell(cell, row, 'cell-center', absolute)}
      {/each}
      {#each tableRow?.getEndVisibleCells() ?? [] as cell, ci (cell.id)}
        {@render bodyCell(cell, row, ci === 0 ? 'cell-end pinned-edge' : 'cell-end', absolute)}
      {/each}
    </tr>
  {/each}
</tbody>

{#snippet bodyCell(cell, row, extraClass, rowIndex)}
  {@const index = cell.column.columnDef.meta?.index ?? 0}
  {@const value = row[index]}
  <td
    class="{cellClass(value)} {extraClass}"
    data-selected={cell.getIsSelected() ? 'true' : 'false'}
    class:focused={cell.getIsFocused()}
    tabindex={cell.getTabIndex()}
    style="width: {widthOf(index)}px; min-width: 80px; {extraClass.includes(
      'cell-start',
    )
      ? `left: ${
          GUTTER_WIDTH + cell.column.getStart('start')
        }px;`
      : ''}{extraClass.includes('cell-end')
      ? `right: ${cell.column.getAfter('end')}px;`
      : ''}"
    onmousedown={(e) => onCellMouseDown(rowIndex, cell.column.id, e)}
    onmouseenter={() => onCellMouseEnter(rowIndex, cell.column.id)}
    oncontextmenu={(e) => onCellContextMenu(e, index, value)}
  >
    {#if typeof value === 'boolean'}
      <span class="bool-badge" class:true={value} class:false={!value}
        >{String(value)}</span
      >
    {:else}
      <span class="cell-content">{formatCell(value)}</span>
    {/if}
  </td>
{/snippet}

<style>
  /* Zebra + hover + row borders */
  tr.even td {
    background: var(--bg-subtle);
  }
  tr:hover td {
    background: var(--bg-hover);
  }
  tr:hover td:first-child {
    box-shadow: inset 3px 0 0 var(--accent);
  }
  td {
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--grid-line);
    border-right: 1px solid var(--grid-line);
    white-space: nowrap;
  }
  td:last-child {
    border-right: none;
  }
  td.row-num {
    text-align: center;
    padding: var(--space-2) 4px;
    width: 44px;
    border-right: 1px solid var(--grid-line);
  }
  td.cell-null {
    color: var(--text-muted);
    font-style: italic;
    text-decoration: underline;
    text-decoration-style: dotted;
    text-underline-offset: 3px;
  }
  td.cell-number {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  td.cell-bool {
    text-align: left;
  }

  /* Pinned columns stick inside the scroller. They need opaque backgrounds or
     centre-group cells scroll out beneath them — keep the zebra/hover tiers
     instead of one flat colour so striping survives pinning. */
  td.cell-start,
  td.cell-end {
    position: sticky;
    z-index: 2;
    background: var(--bg-surface);
  }
  tr.even td.cell-start,
  tr.even td.cell-end {
    background: var(--bg-subtle);
  }
  tr:hover td.cell-start,
  tr:hover td.cell-end {
    background: var(--bg-hover);
  }
  td.pinned-edge {
    box-shadow: 2px 0 6px -2px rgba(0, 0, 0, 0.55);
  }

  /* Cell-range selection. The tint is re-asserted at the zebra and hover
     tiers below: their selectors are more specific than a bare attribute
     selector, and an unpainted-on-striped-rows selection is invisible. */
  td[data-selected='true'],
  tr.even td[data-selected='true'] {
    background: var(--accent-soft, rgba(120, 140, 255, 0.18));
  }
  tr:hover td[data-selected='true'] {
    background: var(--accent-soft-strong, rgba(120, 140, 255, 0.28));
  }

  td.focused {
    outline: 2px solid var(--accent, #6b7cff);
    outline-offset: -2px;
  }

  /* Boolean badges */
  .bool-badge {
    display: inline-block;
    padding: 2px 10px;
    border-radius: var(--radius-full);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
  }
  .bool-badge.true {
    background: var(--success-bg);
    color: var(--success);
  }
  .bool-badge.false {
    background: var(--danger-bg);
    color: var(--danger);
  }

  /* Cell content ellipsis */
  .cell-content {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
