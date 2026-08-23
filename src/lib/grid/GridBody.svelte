<script>
  // formatCell stays here in Lucent's own markup rather than moving into
  // column cell renderers: those need FlexRender in the body, which would
  // break the rule that only engine.svelte.ts touches the adapter. Spec §3.3.
  import { formatCell, cellClass } from './format.js';

  let {
    table,
    pageRows = [],
    pageOffset = 0,
    columnWidths = {},
    checkedRows = new Set(),
    onToggleCheck,
    onCellContextMenu,
  } = $props();

  // Paging is manual (spec D3), so the table's row model holds the whole
  // accumulated buffer while the body renders one page slice of it. Column
  // order still comes from the table.
  const leafColumns = $derived(table.getVisibleLeafColumns());

  function widthOf(index) {
    return columnWidths[index] || 150;
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
      {#each leafColumns as column (column.id)}
        {@const index = column.columnDef.meta?.index ?? 0}
        {@const cell = row[index]}
        <td
          class={cellClass(cell)}
          style="width: {widthOf(index)}px; min-width: 80px;"
          oncontextmenu={(e) => onCellContextMenu(e, index, cell)}
        >
          {#if typeof cell === 'boolean'}
            <span class="bool-badge" class:true={cell} class:false={!cell}
              >{String(cell)}</span
            >
          {:else}
            <span class="cell-content">{formatCell(cell)}</span>
          {/if}
        </td>
      {/each}
    </tr>
  {/each}
</tbody>

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
  td.row-num input {
    cursor: pointer;
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

  /* Checkbox styling (body side; the header copy stays in ResultsGrid
     until Task 8 rewrites it) */
  td input[type='checkbox'] {
    width: 14px;
    height: 14px;
    accent-color: var(--accent);
    cursor: pointer;
  }
</style>
